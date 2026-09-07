//! Binary entry. `--service` runs the headless backup service, `--version` prints
//! the version. Any other invocation is the GUI (Plan 2); until then it prints a hint.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--version") => println!("{}", env!("CARGO_PKG_VERSION")),
        Some("--service") => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            if let Err(e) = rt.block_on(powerpcu_backup::service::run()) {
                eprintln!("service error: {e}");
                std::process::exit(1);
            }
        }
        _ => println!("PowerPCU Backup {}: GUI mode is not built yet. Use --service or --version.", env!("CARGO_PKG_VERSION")),
    }
}
