use std::path::{Path, PathBuf};

pub fn workspace(action: &str, vault_override: Option<&Path>) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    match action {
        "init" => match kodex::workspace::init(&cwd) {
            Ok(path) => println!("Created {}", path.display()),
            Err(e) => eprintln!("Error: {e}"),
        },
        "run" => {
            let config_path = match kodex::workspace::find_config(&cwd) {
                Some(p) => p,
                None => {
                    eprintln!("No kodex-workspace.yaml found. Run `kodex workspace init` first.");
                    return;
                }
            };
            let config = match kodex::workspace::load_config(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Config error: {e}");
                    return;
                }
            };
            if let Err(e) = kodex::workspace::run(&config, vault_override) {
                eprintln!("Workspace error: {e}");
            }
        }
        _ => {
            println!("Usage: kodex workspace <init|run> [--vault <path>]");
        }
    }
}
