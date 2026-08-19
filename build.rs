use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let build_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "unknown".into());

    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=CHAIN_WALLET_BUILD_TIME={build_time}");
    println!("cargo:rustc-env=CHAIN_WALLET_GIT_HASH={git_hash}");
}
