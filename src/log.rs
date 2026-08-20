use std::env;
use std::io::{self, Write};
use std::sync::OnceLock;

static VERBOSE: OnceLock<bool> = OnceLock::new();

pub fn verbose_enabled() -> bool {
    *VERBOSE.get_or_init(|| {
        if cfg!(debug_assertions) {
            return true;
        }
        matches!(
            env::var("CHAIN_WALLET_VERBOSE")
                .ok()
                .as_deref()
                .map(str::trim),
            Some("1") | Some("true") | Some("yes")
        )
    })
}

pub fn step(phase: &str, message: impl AsRef<str>) {
    if verbose_enabled() {
        eprintln!("[chain-wallet][{phase}] {}", message.as_ref());
    }
}

pub fn detail(phase: &str, message: impl AsRef<str>) {
    if verbose_enabled() {
        eprintln!("[chain-wallet][{phase}]   {}", message.as_ref());
    }
}

pub fn warn(phase: &str, message: impl AsRef<str>) {
    eprintln!("[chain-wallet][{phase}][warn] {}", message.as_ref());
}

pub fn error(phase: &str, message: impl AsRef<str>) {
    eprintln!("[chain-wallet][{phase}][error] {}", message.as_ref());
}

pub fn dump_tail(label: &str, path: &std::path::Path, max_lines: usize) {
    let Ok(content) = std::fs::read_to_string(path) else {
        warn(label, format!("log file missing: {}", path.display()));
        return;
    };

    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    error(label, format!("last {} line(s) from {}", lines.len() - start, path.display()));
    for line in &lines[start..] {
        let _ = writeln!(io::stderr(), "  | {line}");
    }
}
