# Candidate Guide — Technical Interview

This session has **two parts**:

1. **~15 minutes** — Rust and blockchain questions (discussion only, no coding required)
2. **~30–45 minutes** — Walk through this project on your machine, run commands, answer questions about what you see

Everything in part two runs **locally**. No cloud account needed.

---

## Before the interview

| Tool | Why |
|------|-----|
| **Rust** (stable) + Cargo | Build `chain-wallet` |
| **Terminal** | PowerShell or bash |
| **This repo** cloned or extracted | Project root contains `chain-wallet/` and `Cargo.toml` |

Optional: skim the repo layout once before the call.

```text
chain-wallet/   ← wallet, local chain, ETH/BTC helpers
contracts/      ← Solidity (not required for the live session)
docs/           ← this file
```

---

## Part 1 — Rust & blockchain warm-up (~15 min)

No laptop demo required unless the interviewer asks you to sketch something. Think out loud.

### Rust topics to review

- **Ownership & borrowing** — why Rust has one mutable reference OR many immutable ones
- **`Result` / `Option`** — error handling instead of exceptions
- **Traits** — shared behaviour (like interfaces)
- **`match` and `?`** — propagating errors
- **Tests** — `#[test]`, `cargo test`, unit vs integration tests
- **Modules & crates** — how a workspace like this repo is organized

### Blockchain topics to review

- **Account model vs UTXO** — this project uses balances per address (account model)
- **Public / private keys** — sign with private; verify with public
- **ECDSA** — common scheme for transaction signatures
- **Nonce** — stops replay; orders transactions per account
- **Mempool** — pending transactions not yet in a block
- **Proof of work** — miners search for a hash meeting a difficulty target
- **Merkle root** — commits to all transactions in a block
- **Block reward & fees** — how miners get paid

### Example questions you might hear

*(Hints only — prepare your own words.)*

| Topic | Hint |
|-------|------|
| Why does Rust borrow instead of garbage-collect? | Predictable memory, no GC pauses, compile-time safety |
| `Result<T, E>` vs panicking | CLI/library code should return errors; panics for bugs |
| What is a nonce on-chain? | Per-account counter; each tx must use the next value |
| UTXO vs account balance | UTXO: spend outputs; account: one balance number per address |
| What does PoW difficulty mean? | Harder = hash must have more leading zeros |
| Double-spend problem | Same coins spent twice; chain + ordering prevents it |

---

## Part 2 — Project walkthrough (~30–45 min)

### Important — set this before `generate`

```powershell
# Windows
$env:CHAIN_WALLET_SKIP_BACKGROUND = "1"
```

```bash
# macOS / Linux
export CHAIN_WALLET_SKIP_BACKGROUND=1
```

This keeps wallet generation **local only**. Always use it during the interview.

Use a dedicated data directory: `--data-dir .chain-data-interview`

---

### Step 1 — Verify setup (~5 min)

```powershell
cd test-rust
$env:CARGO_TARGET_DIR = "$env:TEMP\cw-cargo-target"
$env:CHAIN_WALLET_SKIP_BACKGROUND = "1"
cargo test -p chain-wallet
```

Expect about **18** passing tests (wallet, chain, mempool, tamper checks).

---

### Step 2 — Create two wallets (~3 min)

```powershell
$env:CHAIN_WALLET_SKIP_BACKGROUND = "1"
cargo run -p chain-wallet -- generate    # Alice
cargo run -p chain-wallet -- generate    # Bob
```

Save `private_key`, `public_key`, and `address` (`cw1...`) for each.

---

### Step 3 — Sign & verify (~5 min)

```powershell
cargo run -p chain-wallet -- sign --private-key <ALICE_PK> --message "hello"
cargo run -p chain-wallet -- verify --public-key <ALICE_PUB> --message "hello" --signature <SIG>
cargo run -p chain-wallet -- verify --public-key <ALICE_PUB> --message "hello!" --signature <SIG>
```

**Be ready to explain:** how `cw1` addresses are built; why changing the message breaks verification.

**Hints:** SHA-256 of compressed public key → take 20 bytes → hex → prefix `cw1` (43 chars total).

---

### Step 4 — Run the chain (~15 min)

```powershell
cargo run -p chain-wallet -- chain init --data-dir .chain-data-interview
cargo run -p chain-wallet -- chain mine --miner-address <ALICE_CW1> --data-dir .chain-data-interview
cargo run -p chain-wallet -- chain send --private-key <ALICE_PK> --to <BOB_CW1> --amount 10 --fee 1 --data-dir .chain-data-interview
cargo run -p chain-wallet -- chain mine --miner-address <ALICE_CW1> --data-dir .chain-data-interview
cargo run -p chain-wallet -- chain balance --address <BOB_CW1> --data-dir .chain-data-interview
cargo run -p chain-wallet -- chain balance --address <ALICE_CW1> --data-dir .chain-data-interview
cargo run -p chain-wallet -- chain status --data-dir .chain-data-interview
```

**Be ready to explain:**

| Question | Hint |
|----------|------|
| Bob’s balance after confirm? | One transfer of 10 → **10** |
| Alice’s balance? | Mine (+50) → send (−11) → mine (+50 + fee) → **90** |
| Empty block reward? | **50** coins |
| Pending vs confirmed? | Send sits in mempool until next mine |
| `status` vs `balance`? | Global chain view vs one account |

**Code to know:** `chain-wallet/src/wallet.rs`, `chain-wallet/src/chain.rs`

Transfer signatures use:

```text
chain-testnet-v1|transfer|<from>|<to>|<amount>|<fee>|<nonce>
```

PoW difficulty **3** → block hash hex starts with **`000`**.

State lives in **`state.json`** under your `--data-dir`.

---

### Step 5 — Project code questions (~10 min)

The interviewer may ask you to open specific files. Useful paths:

| File | Topics |
|------|--------|
| `chain-wallet/src/wallet.rs` | Key generation, address, sign/verify |
| `chain-wallet/src/chain.rs` | Mining, mempool, balances, merkle, PoW |
| `chain-wallet/src/main.rs` | CLI structure (clap) |
| `chain-wallet/src/rpc.rs` | HTTP node API |

**Hints:**

- Block reward constant: `BLOCK_REWARD = 50`
- Default fee: `DEFAULT_TX_FEE = 1`
- Self-transfer is rejected in `queue_transfer`
- Balance check includes **pending outgoing** txs (double-spend guard)

---

### Step 6 — Wrap-up (~5 min)

Be ready to discuss:

- What this chain does **not** have (P2P, re-orgs, encrypted wallets, light clients)
- What you would improve for production

---

## CLI cheat sheet

```powershell
$env:CHAIN_WALLET_SKIP_BACKGROUND = "1"

cargo test -p chain-wallet
cargo run -p chain-wallet -- generate
cargo run -p chain-wallet -- sign --private-key <hex> --message "msg"
cargo run -p chain-wallet -- verify --public-key <hex> --message "msg" --signature <hex>

cargo run -p chain-wallet -- chain init --data-dir .chain-data-interview
cargo run -p chain-wallet -- chain mine --miner-address <cw1...> --data-dir .chain-data-interview
cargo run -p chain-wallet -- chain send --private-key <hex> --to <cw1...> --amount 10 --fee 1 --data-dir .chain-data-interview
cargo run -p chain-wallet -- chain balance --address <cw1...> --data-dir .chain-data-interview
cargo run -p chain-wallet -- chain status --data-dir .chain-data-interview

cargo run -p chain-wallet -- --help
cargo run -p chain-wallet -- chain --help
```

---

## Tips

1. Part one is conversational — short clear answers beat long lectures.
2. Part two — run `cargo test` first; write down keys when you `generate`.
3. Mine **after** send before checking balances.
4. Read CLI errors aloud; they usually say “insufficient balance” or “invalid nonce”.
5. Think out loud in part two; the interviewer scores reasoning.

Good luck.
