//! RISEx CLI library crate. The same execution path is shared by CLI
//! invocations, the REPL, the MCP server, and integration tests.
pub mod config;
pub mod errors;
pub mod network;

use clap::{Parser, Subcommand};

use network::Network;
use output::OutputFormat;

pub mod output {
    //! Placeholder re-exported in Task 5. Defined minimally so `Cli` compiles.
    use clap::ValueEnum;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
    pub enum OutputFormat {
        Table,
        Json,
    }
}

/// Runtime context assembled from global CLI flags and config.
pub struct AppContext {
    pub network: Network,
    /// Optional REST base-URL override (`--api-url`); falls back to the network default.
    pub api_url: Option<String>,
    pub format: OutputFormat,
    pub verbose: bool,
    pub force: bool,
}

impl AppContext {
    /// Resolved REST base URL: explicit override or the network default.
    pub fn base_url(&self) -> String {
        self.api_url
            .clone()
            .unwrap_or_else(|| self.network.rest_base().to_string())
    }
}

/// RISEx CLI — trade and query the RISEx perpetuals DEX from the terminal.
#[derive(Parser)]
#[command(name = "risex", version, about, long_about = None)]
pub struct Cli {
    /// Output format: table (default) or json.
    #[arg(short, long, value_enum, global = true)]
    pub output: Option<OutputFormat>,

    /// Show request/response details on stderr.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Network to target.
    #[arg(short, long, value_enum, global = true)]
    pub network: Option<Network>,

    /// Override the REST API base URL.
    #[arg(long, global = true)]
    pub api_url: Option<String>,

    /// Skip confirmation prompts for destructive operations.
    #[arg(long, alias = "force", global = true)]
    pub yes: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Top-level commands. Variants are added per phase.
#[derive(Subcommand)]
pub enum Command {}
