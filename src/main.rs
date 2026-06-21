use std::process;

use clap::Parser;

use risex_cli::network::Network;
use risex_cli::output::OutputFormat;
use risex_cli::{AppContext, Cli};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let format = cli.output.unwrap_or(OutputFormat::Table);

    // Resolution precedence: CLI flag > env var > default.
    let network = cli
        .network
        .or_else(|| {
            std::env::var("RISEX_NETWORK")
                .ok()
                .filter(|s| !s.is_empty())
                .and_then(|s| s.parse::<Network>().ok())
        })
        .unwrap_or_default();
    let api_url = cli.api_url.clone().or_else(|| {
        std::env::var("RISEX_API_URL")
            .ok()
            .filter(|s| !s.is_empty())
    });
    let private_key = cli.private_key.clone().or_else(|| {
        std::env::var("RISEX_PRIVATE_KEY")
            .ok()
            .filter(|s| !s.is_empty())
    });
    let account = cli
        .account
        .clone()
        .or_else(|| std::env::var("RISEX_ACCOUNT").ok().filter(|s| !s.is_empty()));
    let connect_url = cli
        .connect_url
        .clone()
        .or_else(|| std::env::var("RISEX_CONNECT_URL").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "https://connect.risescan.io".to_string());

    let ctx = AppContext {
        network,
        api_url,
        format,
        verbose: cli.verbose,
        force: cli.yes,
        private_key,
        account,
        connect_url,
    };

    match cli.command {
        Some(command) => {
            if let Err(e) = risex_cli::dispatch(&ctx, command).await {
                risex_cli::output::render_error(ctx.format, &e);
                process::exit(1);
            }
        }
        None => {
            use clap::CommandFactory;
            Cli::command().print_help().ok();
            println!();
        }
    }
}
