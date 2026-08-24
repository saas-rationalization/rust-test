use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::wallet::{verify_message, Wallet, WalletError};

pub const DEFAULT_DATA_DIR: &str = ".chain-data";
pub const BLOCK_REWARD: u64 = 50;
pub const DEFAULT_TX_FEE: u64 = 1;
pub const DEFAULT_DIFFICULTY: u32 = 3;
pub const CHAIN_ID: &str = "chain-testnet-v1";
const SNAPSHOT_VERSION: u32 = 2;

#[derive(Debug, Error)]
pub enum ChainError {
    #[error("wallet error: {0}")]
    Wallet(#[from] WalletError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("chain not initialized (run `chain init` first)")]
    NotInitialized,
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    #[error("invalid amount")]
    InvalidAmount,
    #[error("invalid transaction: {0}")]
    InvalidTransaction(String),
    #[error("chain state mismatch: {0}")]
    StateMismatch(String),
    #[error("invalid block at height {height}: {reason}")]
    InvalidBlock { height: u64, reason: String },
    #[error("insufficient balance: have {have}, need {need}")]
    InsufficientBalance { have: u64, need: u64 },
    #[error("invalid nonce: expected {expected}, got {got}")]
    InvalidNonce { expected: u64, got: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Transaction {
    pub from: String,
    pub to: String,
    pub amount: u64,
    #[serde(default)]
    pub fee: u64,
    pub nonce: u64,
    pub public_key: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Block {
    pub height: u64,
    pub prev_hash: String,
    pub timestamp: u64,
    pub miner: String,
    #[serde(default)]
    pub merkle_root: String,
    #[serde(default)]
    pub difficulty: u32,
    #[serde(default)]
    pub pow_nonce: u64,
    pub transactions: Vec<Transaction>,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChainSnapshot {
    #[serde(default = "default_snapshot_version")]
    version: u32,
    chain_id: String,
    #[serde(default = "default_chain_difficulty")]
    difficulty: u32,
    blocks: Vec<Block>,
    balances: HashMap<String, u64>,
    nonces: HashMap<String, u64>,
    pending: Vec<Transaction>,
}

fn default_snapshot_version() -> u32 {
    1
}

fn default_chain_difficulty() -> u32 {
    DEFAULT_DIFFICULTY
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub block_height: u64,
    pub tx_id: String,
    pub direction: HistoryDirection,
    pub counterparty: String,
    pub amount: u64,
    pub fee: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryDirection {
    Sent,
    Received,
    Mined,
}

pub struct ChainStore {
    data_dir: PathBuf,
    difficulty: u32,
    blocks: Vec<Block>,
    balances: HashMap<String, u64>,
    nonces: HashMap<String, u64>,
    pending: Vec<Transaction>,
}

impl ChainStore {
    pub fn init<P: AsRef<Path>>(data_dir: P) -> Result<Self, ChainError> {
        let data_dir = data_dir.as_ref().to_path_buf();
        fs::create_dir_all(&data_dir)?;

        let mut genesis = Block {
            height: 0,
            prev_hash: "0".repeat(64),
            timestamp: unix_now(),
            miner: "genesis".into(),
            merkle_root: merkle_root(&[]),
            difficulty: 0,
            pow_nonce: 0,
            transactions: vec![],
            hash: String::new(),
        };
        genesis.hash = block_hash(&genesis);

        let store = Self {
            data_dir,
            difficulty: DEFAULT_DIFFICULTY,
            blocks: vec![genesis],
            balances: HashMap::new(),
            nonces: HashMap::new(),
            pending: Vec::new(),
        };
        store.save()?;
        Ok(store)
    }

    pub fn open<P: AsRef<Path>>(data_dir: P) -> Result<Self, ChainError> {
        let data_dir = data_dir.as_ref().to_path_buf();
        let path = Self::state_path(&data_dir);
        if !path.exists() {
            return Err(ChainError::NotInitialized);
        }

        let text = fs::read_to_string(path)?;
        let mut snapshot: ChainSnapshot = serde_json::from_str(&text)?;
        if snapshot.chain_id != CHAIN_ID {
            return Err(ChainError::InvalidTransaction(format!(
                "unexpected chain id `{}`",
                snapshot.chain_id
            )));
        }

        let migrated = normalize_snapshot(&mut snapshot)?;

        let store = Self {
            data_dir,
            difficulty: snapshot.difficulty,
            blocks: snapshot.blocks,
            balances: snapshot.balances,
            nonces: snapshot.nonces,
            pending: snapshot.pending,
        };
        store.validate_chain()?;
        if migrated {
            store.save()?;
        }
        Ok(store)
    }

    pub fn open_or_init<P: AsRef<Path>>(data_dir: P) -> Result<Self, ChainError> {
        match Self::open(&data_dir) {
            Ok(store) => Ok(store),
            Err(ChainError::NotInitialized) => Self::init(data_dir),
            Err(err) => Err(err),
        }
    }

    pub fn difficulty(&self) -> u32 {
        self.difficulty
    }

    pub fn height(&self) -> u64 {
        self.blocks.last().map(|block| block.height).unwrap_or(0)
    }

    pub fn tip_hash(&self) -> String {
        self.blocks
            .last()
            .map(|block| block.hash.clone())
            .unwrap_or_else(|| "0".repeat(64))
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn balance(&self, address: &str) -> Result<u64, ChainError> {
        validate_address(address)?;
        Ok(*self.balances.get(address).unwrap_or(&0))
    }

    pub fn nonce(&self, address: &str) -> Result<u64, ChainError> {
        validate_address(address)?;
        Ok(*self.nonces.get(address).unwrap_or(&0))
    }

    pub fn queue_transfer(
        &mut self,
        wallet: &Wallet,
        to: &str,
        amount: u64,
        fee: u64,
    ) -> Result<Transaction, ChainError> {
        validate_address(to)?;
        if amount == 0 {
            return Err(ChainError::InvalidAmount);
        }

        let from = wallet.address();
        if from == to {
            return Err(ChainError::InvalidTransaction(
                "cannot transfer to the same address".into(),
            ));
        }

        let nonce = self.next_transfer_nonce(&from)?;
        let message = transaction_signing_message(&from, to, amount, fee, nonce);
        let tx = Transaction {
            from: from.clone(),
            to: to.to_string(),
            amount,
            fee,
            nonce,
            public_key: wallet.public_key_hex(),
            signature: wallet.sign_message(&message)?,
        };

        let total = amount.saturating_add(fee);
        let balance = self.balance(&from)?;
        let pending_outgoing: u64 = self
            .pending
            .iter()
            .filter(|pending| pending.from == from)
            .map(|pending| pending.amount.saturating_add(pending.fee))
            .sum();
        if balance < total.saturating_add(pending_outgoing) {
            return Err(ChainError::InsufficientBalance {
                have: balance.saturating_sub(pending_outgoing),
                need: total,
            });
        }

        validate_transaction(&tx, nonce)?;
        self.pending.push(tx.clone());
        self.save()?;
        Ok(tx)
    }

    pub fn mine_block(&mut self, miner_address: &str) -> Result<Block, ChainError> {
        validate_address(miner_address)?;

        let tip = self
            .blocks
            .last()
            .cloned()
            .ok_or(ChainError::NotInitialized)?;
        let mut applied = Vec::new();
        let mut fee_total = 0u64;

        let pending: Vec<Transaction> = self.pending.drain(..).collect();
        let mut ordered = pending;
        ordered.sort_by_key(|tx| (tx.from.clone(), tx.nonce));
        for tx in ordered {
            let expected_nonce = self.nonce(&tx.from)?;
            if let Err(err) = validate_transaction(&tx, expected_nonce) {
                eprintln!("dropped invalid pending tx from {}: {err}", tx.from);
                continue;
            }

            let total = tx.amount.saturating_add(tx.fee);
            let from_balance = self.balance(&tx.from)?;
            if from_balance < total {
                eprintln!(
                    "dropped underfunded pending tx from {} (have {from_balance}, need {total})",
                    tx.from
                );
                continue;
            }

            *self.balances.entry(tx.from.clone()).or_insert(0) -= total;
            *self.balances.entry(tx.to.clone()).or_insert(0) += tx.amount;
            *self.nonces.entry(tx.from.clone()).or_insert(0) += 1;
            fee_total = fee_total.saturating_add(tx.fee);
            applied.push(tx);
        }

        let miner_payout = BLOCK_REWARD.saturating_add(fee_total);
        *self.balances.entry(miner_address.to_string()).or_insert(0) += miner_payout;

        let mut block = Block {
            height: tip.height + 1,
            prev_hash: tip.hash,
            timestamp: unix_now(),
            miner: miner_address.to_string(),
            merkle_root: merkle_root(&applied),
            difficulty: self.difficulty,
            pow_nonce: 0,
            transactions: applied,
            hash: String::new(),
        };
        block.pow_nonce = mine_pow_nonce(&block);
        block.hash = block_hash(&block);
        self.blocks.push(block.clone());
        self.save()?;
        Ok(block)
    }

    pub fn history(&self, address: &str) -> Result<Vec<HistoryEntry>, ChainError> {
        validate_address(address)?;
        let mut entries = Vec::new();

        for block in &self.blocks {
            if block.height > 0 && block.miner == address {
                entries.push(HistoryEntry {
                    block_height: block.height,
                    tx_id: format!("reward:{}", block.hash),
                    direction: HistoryDirection::Mined,
                    counterparty: "coinbase".into(),
                    amount: BLOCK_REWARD,
                    fee: block
                        .transactions
                        .iter()
                        .map(|tx| tx.fee)
                        .sum(),
                });
            }

            for tx in &block.transactions {
                if tx.from == address {
                    entries.push(HistoryEntry {
                        block_height: block.height,
                        tx_id: transaction_id(tx),
                        direction: HistoryDirection::Sent,
                        counterparty: tx.to.clone(),
                        amount: tx.amount,
                        fee: tx.fee,
                    });
                } else if tx.to == address {
                    entries.push(HistoryEntry {
                        block_height: block.height,
                        tx_id: transaction_id(tx),
                        direction: HistoryDirection::Received,
                        counterparty: tx.from.clone(),
                        amount: tx.amount,
                        fee: tx.fee,
                    });
                }
            }
        }

        for tx in &self.pending {
            if tx.from == address {
                entries.push(HistoryEntry {
                    block_height: self.height(),
                    tx_id: transaction_id(tx),
                    direction: HistoryDirection::Sent,
                    counterparty: tx.to.clone(),
                    amount: tx.amount,
                    fee: tx.fee,
                });
            }
        }

        Ok(entries)
    }

    pub fn validate_chain(&self) -> Result<(), ChainError> {
        if self.blocks.is_empty() {
            return Err(ChainError::InvalidBlock {
                height: 0,
                reason: "empty chain".into(),
            });
        }

        let genesis = &self.blocks[0];
        if genesis.height != 0 {
            return Err(ChainError::InvalidBlock {
                height: genesis.height,
                reason: "genesis must be height 0".into(),
            });
        }
        validate_block(genesis, None)?;

        for window in self.blocks.windows(2) {
            validate_block(&window[1], Some(&window[0]))?;
        }
        self.validate_account_state()?;
        Ok(())
    }

    fn next_transfer_nonce(&self, address: &str) -> Result<u64, ChainError> {
        let base = self.nonce(address)?;
        let next = self
            .pending
            .iter()
            .filter(|tx| tx.from == address)
            .map(|tx| tx.nonce.saturating_add(1))
            .max()
            .unwrap_or(base);
        Ok(next.max(base))
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn validate_account_state(&self) -> Result<(), ChainError> {
        let mut balances: HashMap<String, u64> = HashMap::new();
        let mut nonces: HashMap<String, u64> = HashMap::new();

        for block in self.blocks.iter().skip(1) {
            let mut block_txs = block.transactions.clone();
            block_txs.sort_by_key(|tx| (tx.from.clone(), tx.nonce));
            for tx in block_txs {
                let expected = *nonces.get(&tx.from).unwrap_or(&0);
                validate_transaction(&tx, expected).map_err(|err| {
                    ChainError::StateMismatch(format!(
                        "block {} tx from {}: {err}",
                        block.height, tx.from
                    ))
                })?;
                let total = tx.amount.saturating_add(tx.fee);
                let from_balance = *balances.get(&tx.from).unwrap_or(&0);
                if from_balance < total {
                    return Err(ChainError::StateMismatch(format!(
                        "block {} would overdraw {} (have {from_balance}, need {total})",
                        block.height, tx.from
                    )));
                }
                *balances.entry(tx.from.clone()).or_insert(0) -= total;
                *balances.entry(tx.to.clone()).or_insert(0) += tx.amount;
                *nonces.entry(tx.from.clone()).or_insert(0) += 1;
            }

            let fee_total: u64 = block.transactions.iter().map(|tx| tx.fee).sum();
            *balances.entry(block.miner.clone()).or_insert(0) +=
                BLOCK_REWARD.saturating_add(fee_total);
        }

        if balances != self.balances {
            return Err(ChainError::StateMismatch(
                "stored balances do not match chain replay".into(),
            ));
        }
        if nonces != self.nonces {
            return Err(ChainError::StateMismatch(
                "stored nonces do not match chain replay".into(),
            ));
        }

        let mut pending_nonces = nonces;
        let mut ordered_pending = self.pending.clone();
        ordered_pending.sort_by_key(|tx| (tx.from.clone(), tx.nonce));
        for tx in ordered_pending {
            let expected = *pending_nonces.get(&tx.from).unwrap_or(&0);
            validate_transaction(&tx, expected).map_err(|err| {
                ChainError::StateMismatch(format!("pending tx from {}: {err}", tx.from))
            })?;
            pending_nonces.insert(tx.from.clone(), expected.saturating_add(1));
        }

        Ok(())
    }

    fn state_path(data_dir: &Path) -> PathBuf {
        data_dir.join("state.json")
    }

    fn save(&self) -> Result<(), ChainError> {
        fs::create_dir_all(&self.data_dir)?;
        let snapshot = ChainSnapshot {
            version: SNAPSHOT_VERSION,
            chain_id: CHAIN_ID.to_string(),
            difficulty: self.difficulty,
            blocks: self.blocks.clone(),
            balances: self.balances.clone(),
            nonces: self.nonces.clone(),
            pending: self.pending.clone(),
        };
        let json = serde_json::to_string_pretty(&snapshot)?;
        fs::write(Self::state_path(&self.data_dir), json)?;
        Ok(())
    }
}

pub fn validate_address(address: &str) -> Result<(), ChainError> {
    if !address.starts_with("cw1") || address.len() != 43 {
        return Err(ChainError::InvalidAddress(address.to_string()));
    }
    if !address[3..].chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(ChainError::InvalidAddress(address.to_string()));
    }
    Ok(())
}

pub fn transaction_signing_message(
    from: &str,
    to: &str,
    amount: u64,
    fee: u64,
    nonce: u64,
) -> String {
    format!("{CHAIN_ID}|transfer|{from}|{to}|{amount}|{fee}|{nonce}")
}

pub fn validate_transaction(tx: &Transaction, expected_nonce: u64) -> Result<(), ChainError> {
    validate_address(&tx.from)?;
    validate_address(&tx.to)?;
    if tx.amount == 0 {
        return Err(ChainError::InvalidAmount);
    }
    if tx.nonce != expected_nonce {
        return Err(ChainError::InvalidNonce {
            expected: expected_nonce,
            got: tx.nonce,
        });
    }

    let derived = Wallet::address_from_public_key_hex(&tx.public_key)?;
    if derived != tx.from {
        return Err(ChainError::InvalidTransaction(
            "public key does not match from address".into(),
        ));
    }

    let message = transaction_signing_message(&tx.from, &tx.to, tx.amount, tx.fee, tx.nonce);
    let valid = verify_message(&message, &tx.signature, &tx.public_key)?;
    if !valid {
        return Err(ChainError::InvalidTransaction(
            "signature verification failed".into(),
        ));
    }
    Ok(())
}

pub fn validate_block(block: &Block, previous: Option<&Block>) -> Result<(), ChainError> {
    if let Some(prev) = previous {
        if block.prev_hash != prev.hash {
            return Err(ChainError::InvalidBlock {
                height: block.height,
                reason: "prev_hash mismatch".into(),
            });
        }
        if block.height != prev.height + 1 {
            return Err(ChainError::InvalidBlock {
                height: block.height,
                reason: "height not sequential".into(),
            });
        }
    }

    if block.merkle_root != merkle_root(&block.transactions) {
        return Err(ChainError::InvalidBlock {
            height: block.height,
            reason: "merkle root mismatch".into(),
        });
    }

    let expected_hash = block_hash(block);
    if block.hash != expected_hash {
        return Err(ChainError::InvalidBlock {
            height: block.height,
            reason: "block hash mismatch".into(),
        });
    }

    if block.height > 0 && !hash_meets_difficulty(&block.hash, block.difficulty) {
        return Err(ChainError::InvalidBlock {
            height: block.height,
            reason: "proof of work failed".into(),
        });
    }

    Ok(())
}

pub fn merkle_root(transactions: &[Transaction]) -> String {
    if transactions.is_empty() {
        return hex::encode([0u8; 32]);
    }

    let mut layer: Vec<String> = transactions.iter().map(transaction_id).collect();
    while layer.len() > 1 {
        let mut next = Vec::new();
        let mut idx = 0;
        while idx < layer.len() {
            let left = &layer[idx];
            let right = layer.get(idx + 1).unwrap_or(left);
            let mut hasher = Sha256::new();
            hasher.update(left.as_bytes());
            hasher.update(right.as_bytes());
            next.push(hex::encode(hasher.finalize()));
            idx += 2;
        }
        layer = next;
    }
    layer[0].clone()
}

pub fn block_hash(block: &Block) -> String {
    let mut hasher = Sha256::new();
    hasher.update(block.height.to_string().as_bytes());
    hasher.update(block.prev_hash.as_bytes());
    hasher.update(block.timestamp.to_string().as_bytes());
    hasher.update(block.miner.as_bytes());
    hasher.update(block.merkle_root.as_bytes());
    hasher.update(block.difficulty.to_string().as_bytes());
    hasher.update(block.pow_nonce.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

pub fn hash_meets_difficulty(hash: &str, difficulty: u32) -> bool {
    hash.chars()
        .take(difficulty as usize)
        .all(|ch| ch == '0')
}

pub fn mine_pow_nonce(block: &Block) -> u64 {
    let mut candidate = block.clone();
    for pow_nonce in 0..u64::MAX {
        candidate.pow_nonce = pow_nonce;
        let hash = block_hash(&candidate);
        if hash_meets_difficulty(&hash, candidate.difficulty) {
            return pow_nonce;
        }
    }
    0
}

pub fn transaction_id(tx: &Transaction) -> String {
    let mut hasher = Sha256::new();
    hasher.update(tx.from.as_bytes());
    hasher.update(tx.to.as_bytes());
    hasher.update(tx.amount.to_string().as_bytes());
    hasher.update(tx.fee.to_string().as_bytes());
    hasher.update(tx.nonce.to_string().as_bytes());
    hasher.update(tx.public_key.as_bytes());
    hasher.update(tx.signature.as_bytes());
    hex::encode(hasher.finalize())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn legacy_block_hash(block: &Block) -> String {
    let mut hasher = Sha256::new();
    hasher.update(block.height.to_string().as_bytes());
    hasher.update(block.prev_hash.as_bytes());
    hasher.update(block.timestamp.to_string().as_bytes());
    hasher.update(block.miner.as_bytes());
    for tx in &block.transactions {
        hasher.update(transaction_id(tx).as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn normalize_block(block: &mut Block, chain_difficulty: u32) -> bool {
    let stored_hash = block.hash.clone();
    let mut changed = false;

    if block.merkle_root.is_empty() {
        block.merkle_root = merkle_root(&block.transactions);
        changed = true;
    }

    if block.height == 0 {
        if block.difficulty != 0 {
            block.difficulty = 0;
            changed = true;
        }
        if block.pow_nonce != 0 {
            block.pow_nonce = 0;
            changed = true;
        }
    } else if block.difficulty == 0 {
        block.difficulty = chain_difficulty.max(DEFAULT_DIFFICULTY);
        changed = true;
    }

    if block_hash(block) == stored_hash {
        return changed;
    }

    if legacy_block_hash(block) == stored_hash {
        if block.height > 0 {
            block.pow_nonce = mine_pow_nonce(block);
        }
        block.hash = block_hash(block);
        return true;
    }

    if block.height > 0 {
        block.pow_nonce = mine_pow_nonce(block);
    } else {
        block.pow_nonce = 0;
    }
    block.hash = block_hash(block);
    true
}

fn normalize_snapshot(snapshot: &mut ChainSnapshot) -> Result<bool, ChainError> {
    let mut migrated = snapshot.version < SNAPSHOT_VERSION;
    if snapshot.difficulty == 0 {
        snapshot.difficulty = DEFAULT_DIFFICULTY;
        migrated = true;
    }

    if snapshot.blocks.is_empty() {
        snapshot.version = SNAPSHOT_VERSION;
        return Ok(migrated);
    }

    if normalize_block(&mut snapshot.blocks[0], snapshot.difficulty) {
        migrated = true;
    }

    for idx in 1..snapshot.blocks.len() {
        let prev_hash = snapshot.blocks[idx - 1].hash.clone();
        if snapshot.blocks[idx].prev_hash != prev_hash {
            snapshot.blocks[idx].prev_hash = prev_hash;
            migrated = true;
        }
        if normalize_block(&mut snapshot.blocks[idx], snapshot.difficulty) {
            migrated = true;
        }
    }

    snapshot.version = SNAPSHOT_VERSION;
    Ok(migrated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::Wallet;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("chain-wallet-{name}-{}", rand::random::<u128>()))
    }

    #[test]
    fn init_and_mine_reward() {
        let dir = temp_dir("init-mine");
        let _ = fs::remove_dir_all(&dir);
        let mut chain = ChainStore::init(&dir).unwrap();
        let miner = Wallet::generate();
        let block = chain.mine_block(&miner.address()).unwrap();
        assert_eq!(block.height, 1);
        assert!(hash_meets_difficulty(&block.hash, block.difficulty));
        assert_eq!(chain.balance(&miner.address()).unwrap(), BLOCK_REWARD);
    }

    #[test]
    fn transfer_with_fee_and_history() {
        let dir = temp_dir("transfer-fee");
        let _ = fs::remove_dir_all(&dir);
        let mut chain = ChainStore::init(&dir).unwrap();
        let alice = Wallet::generate();
        let bob = Wallet::generate();

        chain.mine_block(&alice.address()).unwrap();
        chain
            .queue_transfer(&alice, &bob.address(), 20, DEFAULT_TX_FEE)
            .unwrap();
        chain.mine_block(&alice.address()).unwrap();

        assert_eq!(
            chain.balance(&alice.address()).unwrap(),
            BLOCK_REWARD * 2 - 20 - DEFAULT_TX_FEE + DEFAULT_TX_FEE
        );
        assert_eq!(chain.balance(&bob.address()).unwrap(), 20);

        let history = chain.history(&alice.address()).unwrap();
        assert!(history.iter().any(|entry| entry.direction == HistoryDirection::Sent));
        assert!(history.iter().any(|entry| entry.direction == HistoryDirection::Mined));
    }

    #[test]
    fn rejects_double_spend_in_mempool() {
        let dir = temp_dir("double-spend");
        let _ = fs::remove_dir_all(&dir);
        let mut chain = ChainStore::init(&dir).unwrap();
        let alice = Wallet::generate();
        let bob = Wallet::generate();
        let carol = Wallet::generate();

        chain.mine_block(&alice.address()).unwrap();
        chain
            .queue_transfer(&alice, &bob.address(), 40, DEFAULT_TX_FEE)
            .unwrap();
        let err = chain
            .queue_transfer(&alice, &carol.address(), 40, DEFAULT_TX_FEE)
            .unwrap_err();
        assert!(matches!(err, ChainError::InsufficientBalance { .. }));
    }

    #[test]
    fn validate_chain_on_open() {
        let dir = temp_dir("validate-open");
        let _ = fs::remove_dir_all(&dir);
        let mut chain = ChainStore::init(&dir).unwrap();
        let miner = Wallet::generate();
        chain.mine_block(&miner.address()).unwrap();
        drop(chain);

        let reopened = ChainStore::open(&dir).unwrap();
        assert_eq!(reopened.height(), 1);
    }

    #[test]
    fn queues_multiple_pending_with_sequential_nonces() {
        let dir = temp_dir("multi-pending");
        let _ = fs::remove_dir_all(&dir);
        let mut chain = ChainStore::init(&dir).unwrap();
        let alice = Wallet::generate();
        let bob = Wallet::generate();
        let carol = Wallet::generate();

        chain.mine_block(&alice.address()).unwrap();
        chain.mine_block(&alice.address()).unwrap();
        chain
            .queue_transfer(&alice, &bob.address(), 10, DEFAULT_TX_FEE)
            .unwrap();
        chain
            .queue_transfer(&alice, &carol.address(), 10, DEFAULT_TX_FEE)
            .unwrap();
        assert_eq!(chain.pending_count(), 2);
        chain.mine_block(&alice.address()).unwrap();
        assert_eq!(chain.balance(&bob.address()).unwrap(), 10);
        assert_eq!(chain.balance(&carol.address()).unwrap(), 10);
    }

    #[test]
    fn migrates_legacy_snapshot_format() {
        let dir = temp_dir("legacy-migrate");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("state.json"),
            include_str!("../../.chain-data/state.json"),
        )
        .unwrap();

        let chain = ChainStore::open(&dir).unwrap();
        assert_eq!(chain.height(), 1);
        assert_eq!(
            chain
                .balance("cw1abcaa0b04ae9ca01508809ed88102accbb03f496")
                .unwrap(),
            50
        );
        drop(chain);
        assert!(ChainStore::open(&dir).is_ok(), "migrated snapshot should reload");
    }

    #[test]
    fn rejects_self_transfer() {
        let dir = temp_dir("self-transfer");
        let _ = fs::remove_dir_all(&dir);
        let mut chain = ChainStore::init(&dir).unwrap();
        let alice = Wallet::generate();
        chain.mine_block(&alice.address()).unwrap();
        let err = chain
            .queue_transfer(&alice, &alice.address(), 1, DEFAULT_TX_FEE)
            .unwrap_err();
        assert!(matches!(err, ChainError::InvalidTransaction(_)));
    }
}
