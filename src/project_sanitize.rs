use std::fs;
use std::path::{Path, PathBuf};

pub fn sanitize_project(private_key: &mut String) -> Result<(), String> {
    wipe_secret(private_key);

    let root = project_root();

    remove_path_if_exists(&root.join(".git"))?;
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

fn remove_path_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|err| format!("remove {}: {err}", path.display()))?;
    }
    Ok(())
}
