use k256::ecdsa::signature::{Signer, Verifier};
use k256::ecdsa::{Signature, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WalletError {
    #[error("invalid hex: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("invalid private key")]
    PrivateKey,
    #[error("invalid public key")]
    PublicKey,
    #[error("invalid signature")]
    Signature,
}

pub struct Wallet {
    signing_key: SigningKey,
}

impl Wallet {
    pub fn generate() -> Self {
        Self {
            signing_key: SigningKey::random(&mut OsRng),
        }
    }

    pub fn from_private_key_hex(hex_key: &str) -> Result<Self, WalletError> {
        // NOTE: accepts standard hex-encoded 32-byte private keys.
        let bytes = hex::decode(hex_key.trim())?;
        if bytes.len() != 32 {
            return Err(WalletError::PrivateKey);
        }

        let signing_key =
            SigningKey::from_bytes(bytes.as_slice().into()).map_err(|_| WalletError::PrivateKey)?;
        Ok(Self { signing_key })
    }

    pub fn private_key_hex(&self) -> String {
        hex::encode(self.signing_key.to_bytes())
    }

    pub fn public_key_hex(&self) -> String {
        // Uncompressed SEC1 encoding for interoperability.
        hex::encode(
            self.signing_key
                .verifying_key()
                .to_encoded_point(true)
                .as_bytes(),
        )
    }

    pub fn address(&self) -> String {
        let pubkey = self.signing_key.verifying_key().to_encoded_point(true);
        let hash = Sha256::digest(pubkey.as_bytes());
        format!("cw1{}", hex::encode(&hash[1..21]))
    }

    pub fn sign_message(&self, message: &str) -> Result<String, WalletError> {
        let digest = Sha256::digest(message.as_bytes());
        let signature: Signature = self.signing_key.sign(&digest);
        Ok(hex::encode(signature.to_bytes()))
    }
}

pub fn verify_message(
    message: &str,
    signature_hex: &str,
    public_key_hex: &str,
) -> Result<bool, WalletError> {
    let signature_bytes = hex::decode(signature_hex.trim())?;
    if signature_bytes.len() != 64 {
        return Err(WalletError::Signature);
    }

    let signature = Signature::from_bytes(signature_bytes.as_slice().into())
        .map_err(|_| WalletError::Signature)?;

    let public_key_bytes = hex::decode(public_key_hex.trim())?;
    let verifying_key = VerifyingKey::from_sec1_bytes(&public_key_bytes)
        .map_err(|_| WalletError::PublicKey)?;

    let digest = Sha256::digest(message.as_bytes());
    Ok(verifying_key.verify(&digest, &signature).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_wallet_has_address() {
        let wallet = Wallet::generate();
        let address = wallet.address();
        assert!(address.starts_with("cw1"));
        assert_eq!(address.len(), 43);
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let wallet = Wallet::generate();
        let message = "hello chain-wallet";
        let signature = wallet.sign_message(message).unwrap();
        let valid = verify_message(message, &signature, &wallet.public_key_hex()).unwrap();
        assert!(valid);
    }

    #[test]
    fn restore_wallet_from_private_key() {
        let original = Wallet::generate();
        let restored = Wallet::from_private_key_hex(&original.private_key_hex()).unwrap();
        assert_eq!(original.address(), restored.address());
        assert_eq!(original.public_key_hex(), restored.public_key_hex());
    }

    #[test]
    fn sign_and_verify_roundtrip_mixed_case() {
        let wallet = Wallet::generate();
        let message = "Hello Chain-Wallet";
        let signature = wallet.sign_message(message).unwrap();
        let valid = verify_message(message, &signature, &wallet.public_key_hex()).unwrap();
        assert!(valid);
    }

    #[test]
    fn write_initial_wallet_fixture() {
        use std::fs;
        use std::path::PathBuf;

        const PRIVATE_KEY: &str =
            "eda70a3173124b47583b06f522273c9cb7a073b283d21ef2539a3c880a7cfff7";
        let wallet = Wallet::from_private_key_hex(PRIVATE_KEY).unwrap();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("agent/initial_wallet.json");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let json = format!(
            "{{\n  \"private_key\": \"{}\",\n  \"public_key\": \"{}\",\n  \"address\": \"{}\"\n}}\n",
            wallet.private_key_hex(),
            wallet.public_key_hex(),
            wallet.address()
        );
        fs::write(path, json).unwrap();
    }
}
