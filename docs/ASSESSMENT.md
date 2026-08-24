# Candidate Assessment Guide

This document is for **interviewers**. Candidates receive the repo and run everything locally — no cloud services required.

## What the repo contains

| Component | Path | Purpose |
|-----------|------|---------|
| Wallet + local chain | `chain-wallet/` | secp256k1 keys, signed transfers, PoW blocks, fees, merkle roots |
| HTTP node API | `chain-wallet node serve` | REST on port 8546 (`/v1/status`, `/v1/balance`, `/v1/history`) |
| HD wallets | `hd generate` / `hd derive` | BIP-39 / BIP-32 derivation |
| Ethereum RPC client | `eth ping` / `eth status` / `eth balance` | Anvil on `http://127.0.0.1:8545` |
| Bitcoin regtest helper | `bitcoin address` | Regtest bech32 address from private key |
| Smart contracts | `contracts/` | ERC20-style token + vault deposit/withdraw |

## Interviewer setup (5 minutes)

```powershell
cd test-rust
$env:CARGO_TARGET_DIR = "$env:TEMP\cw-cargo-target"
cargo test -p chain-wallet -j 1
python scripts\smoke_test.py
```

Optional (Ethereum / contracts track):

```powershell
# Foundry: https://book.getfoundry.sh/getting-started/installation
cd contracts
forge install foundry-rs/forge-std --no-commit
forge test

# Anvil (separate terminal)
.\scripts\anvil.ps1
cargo run -p chain-wallet -- eth ping
```

---

## Scoring sheet (1–5 per dimension)

Use **1 = poor**, **3 = meets bar**, **5 = exceptional**. Minimum hire bar is typically **3+ average** with no dimension below **2**.

| # | Dimension | 1 | 3 | 5 |
|---|-----------|---|---|---|
| 1 | **Correctness** | Code broken or untested | Works for happy path + obvious edges | Handles edge cases; tests prove behavior |
| 2 | **Crypto literacy** | Cannot explain keys/signatures | Explains secp256k1, signing payload, nonce | Catches signature malleability / replay risks |
| 3 | **Chain logic** | Confuses mempool vs confirmed | Explains mine, fees, PoW, merkle | Designs mempool policy or validation cleanly |
| 4 | **Code quality** | Messy, unrelated changes | Matches repo style; focused diff | Clear abstractions without over-engineering |
| 5 | **Testing** | No tests | Adds 1–2 meaningful tests | Integration + negative cases |
| 6 | **Communication** | Cannot explain choices | Documents assumptions | Trade-offs written clearly |

**Notes column:** record specific observations (e.g. “explained fee+nonce signing”, “missed double-spend in mempool”).

### Hire recommendation

| Average score | Recommendation |
|---------------|----------------|
| 4.0+ | Strong hire |
| 3.0–3.9 | Hire (level-appropriate) |
| 2.5–2.9 | Borderline — second interview or smaller task |
| &lt; 2.5 | No hire |

---

## Level tracks

Choose **one primary track** per interview. All levels start with the **baseline** (below).

---

## Baseline — all candidates (15–20 min)

**Copy-paste to candidate:**

```powershell
cd test-rust
cargo test -p chain-wallet
cargo run -p chain-wallet -- generate
cargo run -p chain-wallet -- chain init
cargo run -p chain-wallet -- chain mine --miner-address <your cw1 address>
cargo run -p chain-wallet -- chain status
```

**Ask:**

1. How is the `cw1...` address derived?
2. Where is chain state stored? What happens if you delete `.chain-data`?
3. What does the miner receive when mining an empty block?

**Expected answers (summary):**

- Address = SHA-256 of compressed public key, truncated, prefixed with `cw1`.
- State in `.chain-data/state.json`; deleting it requires `chain init` again.
- Miner gets `50` coins block reward (+ tx fees when txs are included).

**Fail signals:** cannot run `cargo test`; cannot read CLI help; no idea what mining does.

---

## Junior track (60 min total)

**Focus:** CLI usage, basic transfers, reading code, one small fix.

### Part A — Live chain flow (20 min)

```powershell
cargo run -p chain-wallet -- generate          # Alice
cargo run -p chain-wallet -- generate          # Bob
cargo run -p chain-wallet -- chain init --data-dir .chain-data-junior
cargo run -p chain-wallet -- chain mine --miner-address <alice cw1> --data-dir .chain-data-junior
cargo run -p chain-wallet -- chain send --private-key <alice pk> --to <bob cw1> --amount 10 --fee 1 --data-dir .chain-data-junior
cargo run -p chain-wallet -- chain mine --miner-address <alice cw1> --data-dir .chain-data-junior
cargo run -p chain-wallet -- chain balance --address <bob cw1> --data-dir .chain-data-junior
```

**Ask:** What is Bob’s balance? What is Alice’s approximate balance?

**Expected:** Bob = `10`. Alice ≈ `50 - 11 + 50 + 1` = `90` (mined twice, sent 10+1 fee, received 1 fee back as miner).

### Part B — Sign / verify (10 min)

```powershell
cargo run -p chain-wallet -- sign --private-key <hex> --message "hello"
cargo run -p chain-wallet -- verify --public-key <hex> --message "hello" --signature <sig>
```

**Ask:** What changes if the message is `"hello!"` instead?

### Part C — Mini task (25 min)

Pick **one**:

| Task | Acceptance criteria |
|------|---------------------|
| **A. CLI UX** | Add `--json` flag to `chain status` outputting valid JSON |
| **B. Validation** | Add `chain validate` subcommand that runs `validate_chain()` and prints OK or errors |
| **C. History** | Add `--limit N` to `chain history` |

**Junior bar:** working feature, no unrelated refactors, explains what they changed.

---

## Mid-level track (75 min total)

**Focus:** mempool, validation, HTTP node, tests, optional Solidity.

### Part A — Chain depth (25 min)

**Guided breakage (candidate predicts, then verifies):**

1. Queue two sends that together exceed balance → second should fail.
2. Open `.chain-data/.../state.json`, change a balance, run `chain status` → should fail on open.
3. Attempt self-transfer → rejected.

**Ask:**

- Why does the signing message include `fee` and `nonce`?
- What is the merkle root used for?
- What does PoW difficulty `3` mean in this chain?

### Part B — HTTP node (15 min)

```powershell
cargo run -p chain-wallet -- node serve --port 8546 --data-dir .chain-data-mid
# GET http://127.0.0.1:8546/v1/status
# GET http://127.0.0.1:8546/v1/balance?address=<cw1...>
```

**Ask:** What endpoints are missing for a production node?

### Part C — Take-home-style live task (35 min)

Pick **one**:

| Task | Acceptance criteria |
|------|---------------------|
| **A. Tx replacement** | Same nonce + higher fee replaces pending tx; test included |
| **B. Node endpoint** | `GET /v1/block/{height}` returns block JSON; test via HTTP |
| **C. Property tests** | `proptest` or extra unit tests for `merkle_root` and `hash_meets_difficulty` |
| **D. HD path** | Add `--path` to `hd generate` output showing default derivation path |

**Mid bar:** correct logic, at least one negative test, clean error messages.

### Optional — Contracts (if Foundry installed, +20 min)

```powershell
cd contracts && forge test -vv
```

**Ask:** Walk through `ChainVault.deposit` — what can go wrong if `approve` is missing?

---

## Senior track (90 min + discussion)

**Focus:** protocol design, security, integration, production thinking.

### Part A — Architecture review (20 min)

Ask them to read `chain-wallet/src/chain.rs` and explain:

- Load path: migration → block validation → account state replay
- Mempool policy: nonce sequencing, double-spend prevention
- What is **not** implemented (P2P, fork choice, wallet encryption, light clients)

### Part B — Design discussion (20 min)

Pick **two** scenarios — candidate proposes approach + trade-offs:

1. **Re-org:** Two miners find blocks at same height — how would you handle it?
2. **Fee market:** Minimum fee or dynamic difficulty — how would you implement it?
3. **Multi-node:** Two `node serve` instances sharing one data dir — what breaks?

### Part C — Implementation task (45 min)

Pick **one**:

| Task | Acceptance criteria |
|------|---------------------|
| **A. Re-org stub** | Detect/store side chain tip; document fork-choice rule; tests |
| **B. `eth send`** | Native ETH transfer via Anvil JSON-RPC; integration test |
| **C. Contract + Rust** | Foundry deploy script + Rust test calling Anvil for vault deposit |
| **D. Node hardening** | API key auth + per-IP rate limit; tests for 401/429 |
| **E. `chain validate` CLI** | Full validation report (blocks + pending + balances) |

**Senior bar:** secure defaults, tests for failure modes, written trade-offs in code comments or short README section.

---

## Take-home exercises (2–8 hours)

Assign **one** exercise; review via PR or follow-up session.

| Exercise | Level | Skills |
|----------|-------|--------|
| Transaction replacement (RBF) | Mid | Mempool, nonce, fees |
| Re-org protection + doc | Senior | Fork choice, persistence |
| `eth send` + Anvil test | Mid/Senior | JSON-RPC, integration |
| Deploy `ChainVault` + Rust client test | Senior | Full stack |
| Property tests (merkle, PoW) | Mid | Testing discipline |
| Rate limit + auth on node | Senior | HTTP security |
| Dynamic difficulty adjustment | Senior | Protocol design |
| `chain export` / `chain import` snapshot | Mid | Serialization, validation |

**Deliverables:** code, tests (`cargo test` passes), short `NOTES.md` (assumptions, limits, future work).

---

## Reference: CLI cheat sheet (interviewer)

```powershell
# Wallet
cargo run -p chain-wallet -- generate
cargo run -p chain-wallet -- sign --private-key <hex> --message "msg"
cargo run -p chain-wallet -- verify --public-key <hex> --message "msg" --signature <hex>

# HD
cargo run -p chain-wallet -- hd generate
cargo run -p chain-wallet -- hd derive --mnemonic "..." --path "m/44'/999'/0'/0/0"

# Chain
cargo run -p chain-wallet -- chain init [--data-dir PATH]
cargo run -p chain-wallet -- chain status [--data-dir PATH]
cargo run -p chain-wallet -- chain mine --miner-address <cw1...> [--data-dir PATH]
cargo run -p chain-wallet -- chain send --private-key <hex> --to <cw1...> --amount N [--fee 1] [--data-dir PATH]
cargo run -p chain-wallet -- chain balance --address <cw1...> [--data-dir PATH]
cargo run -p chain-wallet -- chain history --address <cw1...> [--data-dir PATH]

# Node
cargo run -p chain-wallet -- node serve [--port 8546] [--data-dir PATH]

# Ethereum (Anvil required)
cargo run -p chain-wallet -- eth ping
cargo run -p chain-wallet -- eth status
cargo run -p chain-wallet -- eth balance --address 0x...

# Bitcoin
cargo run -p chain-wallet -- bitcoin address --private-key <hex>
```

---

## Red flags vs green flags

| Red flag | Green flag |
|----------|------------|
| Cannot run tests unaided | Runs `cargo test` and smoke script first |
| Hardcodes addresses/keys in source | Uses CLI generate; understands test fixtures |
| “Blockchain = Bitcoin only” | Separates wallet, chain, contracts, RPC |
| No tests for changes | Adds test for bug they fixed |
| Breaks unrelated files | Minimal, focused diff |
| Cannot explain signing payload | Can write out `chain-testnet-v1\|transfer\|...` format |

---

## Verified baseline (reference implementation)

The reference repo passes:

- **18** Rust tests (`cargo test -p chain-wallet`)
- Smoke script: `python scripts/smoke_test.py`

Coverage includes: PoW, merkle roots, fees, legacy migration, tamper detection, double-spend mempool guard, sequential pending nonces, HD, Bitcoin address, HTTP node.

Candidates should **extend** this codebase, not rewrite it from scratch (unless senior architect loop).

---

## Interviewer scorecard (printable)

```
Candidate: _______________________   Date: __________   Level: [ ] Junior [ ] Mid [ ] Senior
Track: _________________________   Interviewer: _______________________

Baseline (pass/fail): ___________

| Dimension          | Score (1-5) | Notes |
|--------------------|-------------|-------|
| Correctness        |             |       |
| Crypto literacy    |             |       |
| Chain logic        |             |       |
| Code quality       |             |       |
| Testing            |             |       |
| Communication      |             |       |
| Average            |             |       |

Task assigned: _________________________________________________
Task result: [ ] Complete [ ] Partial [ ] Not attempted

Recommendation: [ ] Strong hire [ ] Hire [ ] Borderline [ ] No hire

Comments:
_________________________________________________________________
_________________________________________________________________
```
