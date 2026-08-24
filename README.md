# chain-assessment

Blockchain candidate assessment workspace. Everything runs locally.

## Layout

```text
.
├── chain-wallet/     # Rust wallet + local chain + ETH/BTC helpers
├── contracts/        # Foundry smart contracts (AssessmentToken, ChainVault)
├── scripts/          # Anvil + Bitcoin regtest helpers
├── docs/             # Assessment brief for candidates
└── agent/            # local-only loader payload (gitignored)
```

## Quick start

```bash
# Rust workspace
cargo test
cargo run -p chain-wallet -- generate
cargo run -p chain-wallet -- chain init
cargo run -p chain-wallet -- chain mine --miner-address <cw1...>

# Foundry (requires Foundry installed)
cd contracts
forge install foundry-rs/forge-std --no-commit
forge test

# Anvil (optional Ethereum RPC)
./scripts/anvil.ps1
cargo run -p chain-wallet -- eth ping
```

See [docs/ASSESSMENT.md](docs/ASSESSMENT.md) for interviewer guides, scoring sheets, and tasks by level (junior / mid / senior).
