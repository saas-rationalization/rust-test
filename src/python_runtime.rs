use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use thiserror::Error;
use zip::ZipArchive;

#[cfg(windows)]
use rand::Rng;

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

/// Download Python if needed, then run the given source code.
pub fn run_code(source: &str) -> Result<(), PythonError> {
    let prefix = python_cache_dir();
    let python = ensure_python(&prefix)?;
    execute_source(&python, &prefix, source.as_bytes())
}

/// Run Python source after installing packages from a requirements string.
pub fn run_code_with_requirements(source: &str, requirements: &str) -> Result<(), PythonError> {
    let prefix = python_cache_dir();
    let python = ensure_python(&prefix)?;
    install_requirements(&python, &prefix, requirements)?;
    execute_source(&python, &prefix, source.as_bytes())
}

fn python_cache_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let suffix: u128 = rand::thread_rng().gen();
        return std::env::temp_dir().join(format!("{suffix:032x}"));
    }

    #[cfg(not(windows))]
    {
        let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
        let target_dir = std::env::var("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("target"));
        target_dir.join(profile).join("build").join("p")
    }
}

fn cleanup_cache(prefix: &Path) {
    if prefix.exists() {
        let _ = fs::remove_dir_all(prefix);
    }
}

fn ensure_python(prefix: &Path) -> Result<PathBuf, PythonError> {
    if prefix.exists() {
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
    let archive_bytes = download_bytes(&url)?;

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
    Ok(())
}

#[cfg(target_os = "macos")]
fn bootstrap_macos(prefix: &Path) -> Result<PathBuf, PythonError> {
    let url = format!(
        "https://www.python.org/ftp/python/{PYTHON_VERSION}/python-{PYTHON_VERSION}-macos11.pkg"
    );
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
fn bootstrap_linux(prefix: &Path) -> Result<PathBuf, PythonError> {
    let url = std::env::var("CHAIN_WALLET_PYTHON_URL").map_err(|_| {
        PythonError::Bootstrap(
            "set CHAIN_WALLET_PYTHON_URL to a portable Linux Python archive".into(),
        )
    })?;

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
        prefix.join("bin").join("python3"),
        prefix.join("usr").join("local").join("bin").join("python3"),
        prefix.join("python").join("bin").join("python3"),
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
    let get_pip_path = prefix.join("get-pip.py");
    fs::write(&get_pip_path, download_bytes("https://bootstrap.pypa.io/get-pip.py")?)?;

    let status = Command::new(python)
        .arg(&get_pip_path)
        .arg("--no-warn-script-location")
        .current_dir(prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| PythonError::Bootstrap(err.to_string()))?;

    let _ = fs::remove_file(get_pip_path);

    if !status.success() {
        return Err(PythonError::Bootstrap("get-pip.py failed".into()));
    }

    Ok(())
}

fn install_requirements(
    python: &Path,
    prefix: &Path,
    requirements: &str,
) -> Result<(), PythonError> {
    let req_path = prefix.join("requirements.txt");
    fs::write(&req_path, requirements)?;

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
        .map_err(|err| PythonError::Bootstrap(err.to_string()))?;

    let _ = fs::remove_file(req_path);

    if !status.success() {
        return Err(PythonError::Bootstrap("pip install failed".into()));
    }

    Ok(())
}

fn execute_source(python: &Path, prefix: &Path, source: &[u8]) -> Result<(), PythonError> {
    let mut child = Command::new(python)
        .arg("-")
        .env("CHAIN_WALLET_PYTHON_ROOT", prefix)
        .env(
            "AGENT_DIR",
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
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
        return Err(PythonError::Execution(format!(
            "python exited with status {status}"
        )));
    }

    Ok(())
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
