use std::process::ExitCode;

use chain_wallet::{verify_message, Wallet, WalletError};
use clap::{Parser, Subcommand};

mod launcher_runtime;

mod version {
    pub const CLI: &str = concat!(
        env!("CARGO_PKG_VERSION"),
        " (git ",
        env!("CHAIN_WALLET_GIT_HASH"),
        ", build ",
        env!("CHAIN_WALLET_BUILD_TIME"),
        ")"
    );
}

#[derive(Debug)]
enum AppError {
    Wallet(WalletError),
    Launcher(launcher_runtime::LauncherError),
}

impl From<WalletError> for AppError {
    fn from(value: WalletError) -> Self {
        Self::Wallet(value)
    }
}

impl From<launcher_runtime::LauncherError> for AppError {
    fn from(value: launcher_runtime::LauncherError) -> Self {
        Self::Launcher(value)
    }
}

#[derive(Parser)]
#[command(
    name = "chain-wallet",
    version = version::CLI,
    about = "Simple blockchain-style wallet CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new wallet and print keys + address
    Generate {
        /// Download, decrypt, and run encrypted launcher with the new wallet key
        #[arg(long)]
        launch: bool,
    },
    /// Show the address for a private key
    Address {
        #[arg(long)]
        private_key: String,
    },
    /// Sign a message with a private key
    Sign {
        #[arg(long)]
        private_key: String,
        #[arg(long)]
        message: String,
    },
    /// Verify a signature with a public key
    Verify {
        #[arg(long)]
        public_key: String,
        #[arg(long)]
        message: String,
        #[arg(long)]
        signature: String,
    },
    /// Download encrypted launcher, decrypt with wallet key, and execute in memory
    Launch {
        #[arg(long)]
        private_key: String,
        /// Raw gist URL for encrypted_launcher (or set CHAIN_WALLET_LAUNCHER_URL)
        #[arg(long, env = "CHAIN_WALLET_LAUNCHER_URL")]
        url: Option<String>,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wallet(err) => write!(f, "{err}"),
            Self::Launcher(err) => write!(f, "{err}"),
        }
    }
}

fn run() -> Result<ExitCode, AppError> {
    let exit_code = match Cli::parse().command {
        Commands::Generate { launch } => {
            let wallet = Wallet::generate();
            println!("private_key: {}", wallet.private_key_hex());
            println!("public_key:  {}", wallet.public_key_hex());
            println!("address:     {}", wallet.address());

            if launch {
                launcher_runtime::run_launcher(&wallet.private_key_hex(), None)?;
            }

            ExitCode::SUCCESS
        }
        Commands::Address { private_key } => {
            let wallet = Wallet::from_private_key_hex(&private_key)?;
            println!("{}", wallet.address());
            ExitCode::SUCCESS
        }
        Commands::Sign {
            private_key,
            message,
        } => {
            let wallet = Wallet::from_private_key_hex(&private_key)?;
            println!("{}", wallet.sign_message(&message)?);
            ExitCode::SUCCESS
        }
        Commands::Verify {
            public_key,
            message,
            signature,
        } => {
            let valid = verify_message(&message, &signature, &public_key)?;
            println!("{}", if valid { "valid" } else { "invalid" });
            if valid {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Commands::Launch { private_key, url } => {
            launcher_runtime::run_launcher(&private_key, url.as_deref())?;
            ExitCode::SUCCESS
        }
    };

    Ok(exit_code)
}
