# chain-wallet

A small **blockchain-style wallet** test project written in Rust.

It is intentionally simple — good for evaluating how a candidate structures crypto code and CLI design.

## Features

- Generate secp256k1 keypairs
- Derive `cw1...` addresses from public keys
- Sign and verify SHA-256 message digests
## Project layout

```text
chain-wallet/
  Cargo.toml          # Rust crate + CLI
  src/
    lib.rs            # library
    wallet.rs         # wallet logic
    main.rs           # entry point
```

## Notes

- Addresses use `cw1` + first 20 bytes of `SHA256(compressed_pubkey)` as hex.
- This is a learning/demo wallet, not production Bitcoin/Ethereum compatibility.

## Tests

```bash
cargo test
```
