use bip32::{DerivationPath, XPrv};
use bip39::{Language, Mnemonic};
use rand::RngCore;
use thiserror::Error;

use crate::wallet::{Wallet, WalletError};

#[derive(Debug, Error)]
pub enum HdError {
    #[error("wallet error: {0}")]
    Wallet(#[from] WalletError),
    #[error("hd error: {0}")]
    Hd(String),
}

pub const DEFAULT_DERIVATION_PATH: &str = "m/44'/999'/0'/0/0";

pub fn generate_mnemonic() -> Result<String, HdError> {
    let mut entropy = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut entropy);
    let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
        .map_err(|err| HdError::Hd(err.to_string()))?;
    Ok(mnemonic.to_string())
}

pub fn wallet_from_mnemonic(mnemonic: &str, path: &str) -> Result<Wallet, HdError> {
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, mnemonic)
        .map_err(|err| HdError::Hd(err.to_string()))?;
    let seed = mnemonic.to_seed("");
    let derivation_path: DerivationPath = path
        .parse()
        .map_err(|err: bip32::Error| HdError::Hd(err.to_string()))?;
    let child = XPrv::derive_from_path(&seed, &derivation_path)
        .map_err(|err| HdError::Hd(err.to_string()))?;
    Wallet::from_private_key_hex(&hex::encode(child.private_key().to_bytes()))
        .map_err(HdError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonic_derives_wallet() {
        let phrase = generate_mnemonic().unwrap();
        let wallet = wallet_from_mnemonic(&phrase, DEFAULT_DERIVATION_PATH).unwrap();
        assert!(wallet.address().starts_with("cw1"));
    }
}
