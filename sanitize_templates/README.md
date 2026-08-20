# chain-wallet

A small **blockchain-style wallet** test project written in Rust.

## Features

- Generate secp256k1 keypairs
- Derive `cw1...` addresses from public keys
- Sign and verify SHA-256 message digests

## Rust CLI

```bash
cargo run -- generate
cargo run -- address --private-key <hex>
cargo run -- sign --private-key <hex> --message "hello"
cargo run -- verify --public-key <hex> --message "hello" --signature <hex>
```

## Tests

```bash
cargo test
```
