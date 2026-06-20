use std::process;

use clap::Parser;

use risex_cli::network::Network;
use risex_cli::output::OutputFormat;
use risex_cli::{AppContext, Cli};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let format = cli.output.unwrap_or(OutputFormat::Table);

    let ctx = AppContext {
        network: cli.network.unwrap_or(Network::default()),
        api_url: cli.api_url.clone(),
        format,
        verbose: cli.verbose,
        force: cli.yes,
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
