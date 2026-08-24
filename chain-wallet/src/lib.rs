mod bitcoin;
mod chain;
mod eth;
mod hd;
mod rpc;
mod wallet;

pub use bitcoin::{regtest_address_from_private_key_hex, validate_regtest_address, BitcoinError};
pub use chain::{
    block_hash, hash_meets_difficulty, merkle_root, transaction_id, transaction_signing_message,
    validate_address, validate_block, validate_transaction, Block, ChainError, ChainStore,
    HistoryDirection, HistoryEntry, Transaction, BLOCK_REWARD, CHAIN_ID, DEFAULT_DATA_DIR,
    DEFAULT_DIFFICULTY, DEFAULT_TX_FEE,
};
pub use eth::{EthClient, EthError};
pub use hd::{generate_mnemonic, wallet_from_mnemonic, HdError, DEFAULT_DERIVATION_PATH};
pub use rpc::{serve as serve_node, NodeError};
pub use wallet::{verify_message, Wallet, WalletError};

#[cfg(feature = "python")]
mod python_bindings {
    use super::*;
    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;
    use pyo3::wrap_pyfunction;

    #[pyclass(name = "Wallet")]
    pub struct PyWallet {
        inner: Wallet,
    }

    #[pymethods]
    impl PyWallet {
        #[staticmethod]
        fn generate() -> Self {
            Self {
                inner: Wallet::generate(),
            }
        }

        #[staticmethod]
        fn from_private_key(private_key_hex: &str) -> PyResult<Self> {
            Wallet::from_private_key_hex(private_key_hex)
                .map(|inner| Self { inner })
                .map_err(|err| PyValueError::new_err(err.to_string()))
        }

        fn private_key(&self) -> String {
            self.inner.private_key_hex()
        }

        fn public_key(&self) -> String {
            self.inner.public_key_hex()
        }

        fn address(&self) -> String {
            self.inner.address()
        }

        fn sign(&self, message: &str) -> PyResult<String> {
            self.inner
                .sign_message(message)
                .map_err(|err| PyValueError::new_err(err.to_string()))
        }
    }

    #[pyfunction]
    fn py_verify_message(
        message: &str,
        signature_hex: &str,
        public_key_hex: &str,
    ) -> PyResult<bool> {
        verify_message(message, signature_hex, public_key_hex)
            .map_err(|err| PyValueError::new_err(err.to_string()))
    }

    #[pymodule]
    fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<PyWallet>()?;
        m.add_function(wrap_pyfunction!(py_verify_message, m)?)?;
        Ok(())
    }
}
