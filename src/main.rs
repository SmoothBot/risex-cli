use clap::Parser;

use risex_cli::Cli;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(_command) => {
            // Dispatch is wired up in Task 8.
        }
        None => {
            use clap::CommandFactory;
            Cli::command().print_help().ok();
            println!();
        }
    }
}
