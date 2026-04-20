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
        #[arg(long)]
        graph: Option<PathBuf>,
    },
    /// Find shortest path between two nodes
    Path {
        source: String,
        target: String,
        #[arg(long)]
        graph: Option<PathBuf>,
    },
    /// Explain a node and its neighbors
    Explain {
        node: String,
        #[arg(long)]
        graph: Option<PathBuf>,
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
        #[arg()]
        graph: Option<PathBuf>,
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
        #[arg()]
        graph: Option<PathBuf>,
    },
    /// List registered projects
    List,
    /// Forget (delete) knowledge
    Forget {
        /// Title to match (substring)
        #[arg(long)]
        title: Option<String>,
        /// Knowledge type to match
        #[arg(long, name = "type")]
        ktype: Option<String>,
        /// Project name to match
        #[arg(long)]
        project: Option<String>,
        /// Remove entries below this confidence
        #[arg(long)]
        below: Option<f64>,
    },
    /// Import Claude Code memories into kodex
    Import,
    /// Run actor daemon (internal, started by serve)
    Actor,
}

fn resolve_h5(graph: &Option<PathBuf>) -> PathBuf {
    graph.clone().unwrap_or_else(kodex::registry::global_h5)
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
            commands::query::query(&question, dfs, budget, &resolve_h5(&graph));
        }
        Some(Commands::Path {
            source,
            target,
            graph,
        }) => commands::path(&source, &target, &resolve_h5(&graph)),
        Some(Commands::Explain { node, graph }) => commands::explain(&node, &resolve_h5(&graph)),
        Some(Commands::Watch {
            path,
            debounce,
            vault,
        }) => {
            if let Err(e) = kodex::watch::watch(&path, debounce, vault.as_deref()) {
                eprintln!("Watch error: {e}");
            }
        }
        Some(Commands::Benchmark { graph }) => commands::benchmark(&resolve_h5(&graph)),
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
        Some(Commands::Serve { graph }) => commands::serve::serve(&resolve_h5(&graph)),
        Some(Commands::List) => {
            let entries = kodex::registry::list();
            if entries.is_empty() {
                println!("No projects registered. Run `kodex run <path>` first.");
            } else {
                println!("Registered projects ({}):", entries.len());
                for (key, entry) in &entries {
                    println!("  {key}: {}", entry.path.display());
                }
                println!("\nKnowledge: {}", kodex::registry::global_h5().display());
            }
        }
        Some(Commands::Forget {
            title,
            ktype,
            project,
            below,
        }) => {
            let h5 = kodex::registry::global_h5();
            match kodex::storage::forget_knowledge(
                &h5,
                title.as_deref(),
                ktype.as_deref(),
                project.as_deref(),
                below,
            ) {
                Ok(0) => println!("No matching knowledge found."),
                Ok(n) => println!("Removed {n} knowledge entries."),
                Err(e) => eprintln!("Error: {e}"),
            }
        }
        Some(Commands::Import) => {
            let h5 = kodex::registry::global_h5();
            if !h5.exists() {
                eprintln!("No kodex.h5 found. Run `kodex run` first.");
                return;
            }
            match kodex::import::import_claude_memories(&h5) {
                Ok(0) => println!("No memories found to import."),
                Ok(n) => println!("Imported {n} memories from ~/.claude/"),
                Err(e) => eprintln!("Import error: {e}"),
            }
        }
        Some(Commands::Actor) => kodex::actor::run_actor(),
        None => {
            if let Some(path) = cli.path {
                commands::run::run_pipeline(&path);
            } else {
                println!("kodex: no path specified. Use --help for usage.");
            }
        }
    }
}
