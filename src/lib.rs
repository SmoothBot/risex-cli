//! RISEx CLI library crate. The same execution path is shared by CLI
//! invocations, the REPL, the MCP server, and integration tests.
pub mod client;
pub mod commands;
pub mod config;
pub mod errors;
pub mod network;
pub mod output;
pub mod session;
pub mod signing;
pub mod telemetry;

use clap::{Parser, Subcommand};

use commands::market::{self, MarketCommand};
use errors::Result;
use network::Network;
use output::{render, CommandOutput, OutputFormat};

/// Runtime context assembled from global CLI flags and config.
pub struct AppContext {
    pub network: Network,
    /// Optional REST base-URL override (`--api-url`); falls back to the network default.
    pub api_url: Option<String>,
    pub format: OutputFormat,
    pub verbose: bool,
    pub force: bool,
    /// Account private key from flag/env (config is consulted on demand).
    pub private_key: Option<String>,
    /// Account address override from flag/env.
    pub account: Option<String>,
}

impl AppContext {
    /// Resolved REST base URL: explicit override or the network default.
    pub fn base_url(&self) -> String {
        self.api_url
            .clone()
            .unwrap_or_else(|| self.network.rest_base().to_string())
    }

    /// Build a REST client for the resolved base URL.
    pub fn client(&self) -> Result<client::RestClient> {
        client::RestClient::new(&self.base_url())
    }

    /// Resolve trading credentials (flag > env > config).
    pub fn credentials(&self) -> Result<config::Credentials> {
        config::resolve_credentials(self.private_key.as_deref(), self.account.as_deref())
    }

    /// Resolve a signer and the account address it acts for.
    pub fn signer_and_account(&self) -> Result<(signing::Signer, String)> {
        let creds = self.credentials()?;
        let signer = signing::Signer::from_key(creds.private_key.expose())?;
        Ok((signer, creds.account))
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

    /// Account private key (prefer `auth import` / RISEX_PRIVATE_KEY for safety).
    #[arg(long, global = true)]
    pub private_key: Option<String>,

    /// Account address (derived from the key when omitted).
    #[arg(long, global = true)]
    pub account: Option<String>,

    /// Skip confirmation prompts for destructive operations.
    #[arg(short = 'y', long, alias = "force", global = true)]
    pub yes: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Top-level commands. Variants are added per phase.
#[derive(Subcommand)]
pub enum Command {
    /// List markets (optionally filter to one).
    Markets {
        /// Market id or display name (e.g. BTC/USDC).
        #[arg(long)]
        market: Option<String>,
    },
    /// Show last/mark/index price for a market.
    Ticker {
        /// Market id or display name.
        market: String,
    },
    /// Show the order book for a market.
    #[command(alias = "book")]
    Orderbook {
        /// Market id, ticker, or name (btc, bitcoin, BTC/USDC, 1).
        market: String,
        /// Number of price levels per side.
        #[arg(long, default_value = "10")]
        depth: u32,
        /// Aggregate price levels into buckets of this size (quote/USD units).
        /// Defaults to a per-market bucket (~100× the tick); use --no-agg for raw levels.
        #[arg(long, short = 'a')]
        aggregate: Option<f64>,
        /// Disable aggregation and show raw tick-level depth.
        #[arg(long = "no-agg", conflicts_with = "aggregate")]
        no_agg: bool,
        /// Show base amount instead of notional (USD) value.
        #[arg(long)]
        amount: bool,
    },
    /// Show recent trades for a market.
    Trades {
        /// Market id or display name.
        market: String,
        /// Max number of trades.
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Show candles (OHLCV) for a market.
    Candles {
        /// Market id or display name.
        market: String,
        /// Candle resolution (e.g. 1, 5, 60, 1D).
        #[arg(long)]
        resolution: String,
        /// Start time (unix seconds).
        #[arg(long)]
        from: Option<String>,
        /// End time (unix seconds).
        #[arg(long)]
        to: Option<String>,
    },
    /// Show funding-rate info for a market.
    Funding {
        /// Market id or display name.
        market: String,
    },
    /// Show system config (contract addresses, chain, maintenance).
    System,
    /// Manage credentials and the JWT session (import/login/approve/…).
    Auth {
        #[command(subcommand)]
        cmd: commands::auth::AuthCommand,
    },
}

fn build_client(ctx: &AppContext) -> Result<client::RestClient> {
    client::RestClient::new(&ctx.base_url())
}

/// Render-free executor: routes a parsed command to its handler and returns the
/// structured output. Shared by CLI dispatch and (later) the MCP server.
pub async fn execute_command(ctx: &AppContext, command: Command) -> Result<CommandOutput> {
    // Non-market commands route to their own handlers.
    if let Command::Auth { cmd } = command {
        return commands::auth::execute(&cmd, ctx).await;
    }

    let client = build_client(ctx)?;
    let market_cmd = match command {
        Command::Markets { market } => MarketCommand::Markets { market },
        Command::Ticker { market } => MarketCommand::Ticker { market },
        Command::Orderbook {
            market,
            depth,
            aggregate,
            no_agg,
            amount,
        } => MarketCommand::Orderbook {
            market,
            depth,
            aggregate,
            no_agg,
            amount,
        },
        Command::Trades { market, limit } => MarketCommand::Trades { market, limit },
        Command::Candles {
            market,
            resolution,
            from,
            to,
        } => MarketCommand::Candles {
            market,
            resolution,
            from,
            to,
        },
        Command::Funding { market } => MarketCommand::Funding { market },
        Command::System => MarketCommand::System,
        Command::Auth { .. } => unreachable!("handled above"),
    };
    market::execute(&market_cmd, &client, ctx.verbose).await
}

/// Central dispatch: execute a command and render its output.
pub async fn dispatch(ctx: &AppContext, command: Command) -> Result<()> {
    let out = execute_command(ctx, command).await?;
    render(ctx.format, &out);
    Ok(())
}
