//! Shared helpers for write (dangerous) commands.
use std::io::{self, Write};

use colored::Colorize;

use crate::errors::{Result, RisexError};
use crate::network::Network;
use crate::AppContext;

/// Confirm a write action. Prints a summary, a red warning on mainnet, and
/// prompts on stderr unless `--yes` was passed.
pub fn confirm_write(ctx: &AppContext, summary: &str) -> Result<()> {
    eprintln!("{summary}");
    if ctx.network == Network::Mainnet {
        eprintln!(
            "{}",
            "⚠  MAINNET — this uses real funds.".red().bold()
        );
    }
    if ctx.force {
        return Ok(());
    }
    eprint!("Proceed? [y/N] ");
    io::stderr().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).map_err(RisexError::Io)?;
    if !matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Err(RisexError::Validation("aborted by user".into()));
    }
    Ok(())
}
