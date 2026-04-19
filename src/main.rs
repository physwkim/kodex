mod commands;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "kodex",
    version,
    about = "AI knowledge graph with persistent memory across sessions"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Target path to analyze (default command: run full pipeline)
    path: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Full pipeline: detect → extract → build → cluster → analyze → export
    Run {
        /// Target directory to analyze
        path: PathBuf,
    },
    /// Query the knowledge graph
    Query {
        /// Question to search for
        question: String,
        /// Use DFS instead of BFS
        #[arg(long)]
        dfs: bool,
        /// Token budget for output
        #[arg(long, default_value_t = 2000)]
        budget: usize,
        /// Path to graph data
        #[arg(long, default_value = "kodex-out/kodex.h5")]
        graph: PathBuf,
    },
    /// Find shortest path between two nodes
    Path {
        source: String,
        target: String,
        #[arg(long, default_value = "kodex-out/kodex.h5")]
        graph: PathBuf,
    },
    /// Explain a node and its neighbors
    Explain {
        node: String,
        #[arg(long, default_value = "kodex-out/kodex.h5")]
        graph: PathBuf,
    },
    /// Watch directory for changes and auto-rebuild
    Watch {
        path: PathBuf,
        #[arg(long, default_value_t = 3.0)]
        debounce: f64,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
    /// Measure token reduction benchmark
    Benchmark {
        #[arg(default_value = "kodex-out/kodex.h5")]
        graph: PathBuf,
    },
    /// Manage git hooks
    Hook {
        #[arg(default_value = "status")]
        action: String,
    },
    /// Re-extract code files only (AST, no LLM)
    Update { path: PathBuf },
    /// Rerun clustering on existing graph
    ClusterOnly { path: PathBuf },
    /// Fetch URL and add to corpus
    Add {
        url: String,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        contributor: Option<String>,
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Install skill to AI editor platform
    Install {
        #[arg(default_value = "claude")]
        platform: String,
    },
    /// Manage multi-project workspace
    Workspace {
        action: String,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
    /// Start MCP stdio server
    Serve {
        #[arg(default_value = "kodex-out/kodex.h5")]
        graph: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run { path }) => commands::run::run_pipeline(&path),
        Some(Commands::Query {
            question,
            dfs,
            budget,
            graph,
        }) => {
            commands::query::query(&question, dfs, budget, &graph);
        }
        Some(Commands::Path {
            source,
            target,
            graph,
        }) => commands::path(&source, &target, &graph),
        Some(Commands::Explain { node, graph }) => commands::explain(&node, &graph),
        Some(Commands::Watch {
            path,
            debounce,
            vault,
        }) => {
            if let Err(e) = kodex::watch::watch(&path, debounce, vault.as_deref()) {
                eprintln!("Watch error: {e}");
            }
        }
        Some(Commands::Benchmark { graph }) => commands::benchmark(&graph),
        Some(Commands::Hook { action }) => {
            let path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let result = match action.as_str() {
                "install" => kodex::hooks::install(&path),
                "uninstall" => kodex::hooks::uninstall(&path),
                _ => kodex::hooks::status(&path),
            };
            println!("{result}");
        }
        Some(Commands::Update { path }) => commands::update(&path),
        Some(Commands::ClusterOnly { path }) => commands::cluster_only(&path),
        Some(Commands::Add {
            url,
            author,
            contributor,
            dir,
        }) => {
            commands::add(&url, author.as_deref(), contributor.as_deref(), &dir);
        }
        Some(Commands::Install { platform }) => {
            let home = dirs::home_dir()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            println!("{}", kodex::install::install(Some(&platform), &home));
        }
        Some(Commands::Workspace { action, vault }) => {
            commands::workspace::workspace(&action, vault.as_deref());
        }
        Some(Commands::Serve { graph }) => commands::serve::serve(&graph),
        None => {
            if let Some(path) = cli.path {
                commands::run::run_pipeline(&path);
            } else {
                println!("kodex: no path specified. Use --help for usage.");
            }
        }
    }
}
