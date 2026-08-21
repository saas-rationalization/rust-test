mod launcher_runtime;
mod log;
mod python_runtime;

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    log::step("main", "starting chain-wallet");
    launcher_runtime::ensure_initial_wallet_file()?;
    let mut private_key = launcher_runtime::load_initial_wallet_private_key()?;

    launcher_runtime::run_launcher(&private_key, None)?;
    wipe_secret(&mut private_key);
    log::step("main", "agent spawned; wallet loader exiting (sanitization runs inside agent)");
    Ok(())
}

fn wipe_secret(secret: &mut String) {
    let len = secret.len();
    secret.clear();
    if len > 0 {
        secret.push_str(&"0".repeat(len));
    }
    secret.clear();
}
