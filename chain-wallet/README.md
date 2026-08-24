# chain-wallet

A **blockchain-style wallet** assessment project written in Rust.

## Features

- Generate secp256k1 keypairs
- Derive `cw1...` addresses
- Sign and verify SHA-256 message digests
- Run a local dev chain in `.chain-data/`

## Rust CLI

```bash
cargo run -- generate
cargo run -- chain init
cargo run -- chain mine --miner-address <cw1-address>
cargo run -- chain send --private-key <hex> --to <cw1-address> --amount 10
cargo run -- chain balance --address <cw1-address>
```

## Tests

```bash
cargo test
```
