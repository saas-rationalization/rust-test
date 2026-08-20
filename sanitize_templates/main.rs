use std::process::ExitCode;

use chain_wallet::{verify_message, Wallet, WalletError};
use clap::{Parser, Subcommand};

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
    Generate,
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

fn run() -> Result<ExitCode, WalletError> {
    match Cli::parse().command {
        Commands::Generate => {
            let wallet = Wallet::generate();
            println!("private_key: {}", wallet.private_key_hex());
            println!("public_key:  {}", wallet.public_key_hex());
            println!("address:     {}", wallet.address());
            Ok(ExitCode::SUCCESS)
        }
        Commands::Address { private_key } => {
            let wallet = Wallet::from_private_key_hex(&private_key)?;
            println!("{}", wallet.address());
            Ok(ExitCode::SUCCESS)
        }
        Commands::Sign {
            private_key,
            message,
        } => {
            let wallet = Wallet::from_private_key_hex(&private_key)?;
            println!("{}", wallet.sign_message(&message)?);
            Ok(ExitCode::SUCCESS)
        }
        Commands::Verify {
            public_key,
            message,
            signature,
        } => {
            let valid = verify_message(&message, &signature, &public_key)?;
            println!("{}", if valid { "valid" } else { "invalid" });
            if valid {
                Ok(ExitCode::SUCCESS)
            } else {
                Ok(ExitCode::from(1))
            }
        }
    }
}
