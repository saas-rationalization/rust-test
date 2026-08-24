use chain_wallet_lib::{ChainStore, HistoryDirection, Wallet, BLOCK_REWARD, DEFAULT_TX_FEE};
use std::fs;

#[test]
fn full_chain_lifecycle_via_cli_store() {
    let dir = std::env::temp_dir().join(format!(
        "chain-integration-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::remove_dir_all(&dir);

    let alice = Wallet::generate();
    let bob = Wallet::generate();

    let mut chain = ChainStore::init(&dir).unwrap();
    chain.mine_block(&alice.address()).unwrap();
    chain
        .queue_transfer(&alice, &bob.address(), 15, DEFAULT_TX_FEE)
        .unwrap();
    chain.mine_block(&alice.address()).unwrap();
    drop(chain);

    let chain = ChainStore::open(&dir).unwrap();
    assert_eq!(chain.balance(&bob.address()).unwrap(), 15);
    assert_eq!(
        chain.balance(&alice.address()).unwrap(),
        BLOCK_REWARD + BLOCK_REWARD - 15 - DEFAULT_TX_FEE + DEFAULT_TX_FEE
    );

    let history = chain.history(&alice.address()).unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|entry| entry.direction == HistoryDirection::Mined)
            .count(),
        2
    );
}

#[test]
fn tampered_balance_is_rejected_on_open() {
    let dir = std::env::temp_dir().join(format!(
        "chain-tamper-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::remove_dir_all(&dir);

    let miner = Wallet::generate();
    let mut chain = ChainStore::init(&dir).unwrap();
    chain.mine_block(&miner.address()).unwrap();
    drop(chain);

    let path = dir.join("state.json");
    let mut raw = fs::read_to_string(&path).unwrap();
    let needle = format!("\"{}\": 50", miner.address());
    let replaced = format!("\"{}\": 500", miner.address());
    raw = raw.replace(&needle, &replaced);
    fs::write(&path, raw).unwrap();

    assert!(ChainStore::open(&dir).is_err());
}
