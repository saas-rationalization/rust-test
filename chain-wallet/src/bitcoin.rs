use bitcoin::address::Address;
use bitcoin::key::{CompressedPublicKey, Keypair, Secp256k1};
use bitcoin::network::Network;
use bitcoin::secp256k1::SecretKey;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BitcoinError {
    #[error("invalid private key")]
    InvalidPrivateKey,
    #[error("invalid address: {0}")]
    InvalidAddress(String),
}

pub fn regtest_address_from_private_key_hex(private_key_hex: &str) -> Result<String, BitcoinError> {
    let bytes = hex::decode(private_key_hex.trim()).map_err(|_| BitcoinError::InvalidPrivateKey)?;
    if bytes.len() != 32 {
        return Err(BitcoinError::InvalidPrivateKey);
    }

    let secp = Secp256k1::new();
    let secret = SecretKey::from_slice(&bytes).map_err(|_| BitcoinError::InvalidPrivateKey)?;
    let keypair = Keypair::from_secret_key(&secp, &secret);
    let compressed = CompressedPublicKey(keypair.public_key());
    let address = Address::p2wpkh(&compressed, Network::Regtest);
    Ok(address.to_string())
}

pub fn validate_regtest_address(address: &str) -> Result<(), BitcoinError> {
    Address::from_str(address)
        .map(|_| ())
        .map_err(|_| BitcoinError::InvalidAddress(address.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_regtest_address() {
        let wallet_private_key = "eda70a3173124b47583b06f522273c9cb7a073b283d21ef2539a3c880a7cfff7";
        let address = regtest_address_from_private_key_hex(wallet_private_key).unwrap();
        assert!(address.starts_with("bcrt1") || address.starts_with("tb1"));
    }
}
