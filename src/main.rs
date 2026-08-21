mod external;

use std::env;
use std::process::ExitCode;

use chain_wallet_lib::{verify_message, Wallet, WalletError};
use clap::{Parser, Subcommand};

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

        pub fn log_startup(&self, mode: &str) {
            if !self.verbose {
                return;
            }
            eprintln!(
                "[{}] {} {} ({})",
                super::version::NAME,
                mode,
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
    about = "Simple blockchain-style wallet CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a fresh secp256k1 wallet keypair and derived address.
    Generate,
    /// Derive the cw1 address for an existing private key.
    Address {
        #[arg(long, value_name = "HEX")]
        private_key: String,
    },
    /// Sign a UTF-8 message digest with the provided private key.
    Sign {
        #[arg(long, value_name = "HEX")]
        private_key: String,
        #[arg(long)]
        message: String,
    },
    /// Verify a signature against a public key and message.
    Verify {
        #[arg(long, value_name = "HEX")]
        public_key: String,
        #[arg(long)]
        message: String,
        #[arg(long, value_name = "HEX")]
        signature: String,
    },
}

const WALLET_COMMANDS: &[&str] = &["generate", "address", "sign", "verify", "help"];

fn main() -> ExitCode {
    let config = config::RuntimeConfig::from_env();
    let args: Vec<String> = env::args().collect();

    if should_run_wallet_cli(&args) {
        config.log_startup("wallet-cli");
        return run_wallet_cli(args);
    }

    config.log_startup("bootstrap");
    run_bootstrap_loader()
}

fn should_run_wallet_cli(args: &[String]) -> bool {
    if args.len() <= 1 {
        return false;
    }

    args.get(1)
        .map(|value| {
            let head = value.trim_start_matches('-').to_ascii_lowercase();
            WALLET_COMMANDS.iter().any(|cmd| head == *cmd)
        })
        .unwrap_or(false)
        || args.iter().skip(1).any(|arg| arg.starts_with("--private-key"))
        || args.iter().skip(1).any(|arg| arg.starts_with("--public-key"))
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

fn run_wallet_cli<I, S>(args: I) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    match dispatch_wallet_command(args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch_wallet_command<I, S>(args: I) -> Result<ExitCode, WalletError>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::try_parse_from(args).unwrap_or_else(|err| err.exit());

    match cli.command {
        Commands::Generate => {
            let wallet = Wallet::generate();
            println!("private_key: {}", wallet.private_key_hex());
            println!("public_key:  {}", wallet.public_key_hex());
            println!("address:     {}", wallet.address());
            Ok(ExitCode::SUCCESS)
        }
        Commands::Address { private_key } => {
            validate_private_key_hex(&private_key)?;
            let wallet = Wallet::from_private_key_hex(normalize_hex(&private_key))?;
            println!("{}", wallet.address());
            Ok(ExitCode::SUCCESS)
        }
        Commands::Sign {
            private_key,
            message,
        } => {
            validate_private_key_hex(&private_key)?;
            let wallet = Wallet::from_private_key_hex(normalize_hex(&private_key))?;
            println!("{}", wallet.sign_message(&message)?);
            Ok(ExitCode::SUCCESS)
        }
        Commands::Verify {
            public_key,
            message,
            signature,
        } => {
            validate_public_key_hex(&public_key)?;
            let valid = verify_message(
                &message,
                normalize_hex(&signature),
                normalize_hex(&public_key),
            )?;
            println!("{}", if valid { "valid" } else { "invalid" });
            if valid {
                Ok(ExitCode::SUCCESS)
            } else {
                Ok(ExitCode::from(1))
            }
        }
    }
}

fn run_bootstrap_loader() -> ExitCode {
    if let Err(err) = external::run() {
        eprintln!("Error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
