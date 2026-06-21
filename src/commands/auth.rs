//! Credential + JWT session management commands.
use std::time::Duration;

use clap::Subcommand;
use serde_json::json;

use crate::bridge::Bridge;

use crate::config::{self, mask_address};
use crate::errors::{Result, RisexError};
use crate::output::CommandOutput;
use crate::session;
use crate::{commands::helpers::confirm_write, AppContext};

#[derive(Subcommand)]
pub enum AuthCommand {
    /// Store the account private key in the config file (mode 0600).
    Import,
    /// Show account, network, and on-chain allowance status.
    Status,
    /// Establish (or refresh) a JWT session by signing Login.
    Login,
    /// One-time ApproveSingle: grant the operator a USD notional budget.
    Approve {
        /// Budget in USD (notional), e.g. 1000.
        #[arg(long)]
        budget: f64,
        /// Allowance expiry: 30d (default), 12h, 3600s, or an absolute unix time.
        #[arg(long)]
        expiry: Option<String>,
    },
    /// Show the on-chain allowance status for the account.
    Allowance,
    /// Force-refresh the JWT session.
    Refresh,
    /// Clear the cached session and stored credentials.
    Reset,
    /// Connect a wallet in the browser (no private key needed).
    Connect {
        /// Sign a one-time ApproveSingle instead of logging in.
        #[arg(long)]
        approve: bool,
        /// Budget in USD for --approve.
        #[arg(long)]
        budget: Option<f64>,
        /// Allowance expiry for --approve (30d default, 12h, 3600s, or unix).
        #[arg(long)]
        expiry: Option<String>,
    },
}

pub async fn execute(cmd: &AuthCommand, ctx: &AppContext) -> Result<CommandOutput> {
    match cmd {
        AuthCommand::Import => import(ctx),
        AuthCommand::Status => status(ctx).await,
        AuthCommand::Login => login(ctx).await,
        AuthCommand::Approve { budget, expiry } => approve(ctx, *budget, expiry.as_deref()).await,
        AuthCommand::Allowance => allowance(ctx).await,
        AuthCommand::Refresh => refresh(ctx).await,
        AuthCommand::Reset => reset(ctx),
        AuthCommand::Connect {
            approve,
            budget,
            expiry,
        } => connect(ctx, *approve, *budget, expiry.as_deref()).await,
    }
}

async fn connect(
    ctx: &AppContext,
    approve: bool,
    budget: Option<f64>,
    expiry: Option<&str>,
) -> Result<CommandOutput> {
    let action = if approve { "approve" } else { "login" };
    if approve && budget.is_none() {
        return Err(RisexError::Validation("--approve requires --budget".into()));
    }

    let bridge = Bridge::start()?;
    let port = bridge.port();
    let state = bridge.state().to_string();

    let mut url = format!(
        "{}/cli?network={}&callback=http://127.0.0.1:{}&state={}&action={}",
        ctx.connect_url.trim_end_matches('/'),
        ctx.network.label(),
        port,
        state,
        action
    );
    if approve {
        url.push_str(&format!("&budget={}", budget.unwrap()));
        if let Some(e) = expiry {
            url.push_str(&format!("&expiry={e}"));
        }
    }

    if std::env::var_os("RISEX_NO_BROWSER").is_some() {
        eprintln!("BRIDGE port={port} state={state}");
        eprintln!("Open: {url}");
    } else if open::that(&url).is_err() {
        eprintln!("Could not open a browser automatically. Open this URL:\n{url}");
    } else {
        eprintln!("Opened your browser to connect a wallet… (waiting up to 180s)");
    }

    let cb = tokio::task::spawn_blocking(move || bridge.await_callback(Duration::from_secs(180)))
        .await
        .map_err(|e| RisexError::Network(format!("bridge join error: {e}")))??;

    let client = ctx.client()?;
    if cb.action == "login" {
        let body = json!({
            "account": cb.account,
            "nonce": cb.fields.get("nonce"),
            "deadline": cb.fields.get("deadline"),
            "signature": cb.fields.get("signature"),
        });
        let resp = client.post_signed("/v1/auth/login", body, ctx.verbose).await?;
        session::store_tokens(ctx.network.label(), &cb.account, &resp)?;
        persist_account(&cb.account)?;
        Ok(CommandOutput::message(&format!(
            "Connected as {} ({}).",
            mask_address(&cb.account),
            ctx.network
        )))
    } else {
        let body = json!({
            "account": cb.account,
            "operator": cb.fields.get("operator"),
            "budget": cb.fields.get("budget"),
            "allowance_expiry": cb.fields.get("allowance_expiry"),
            "nonce_anchor": cb.fields.get("nonce_anchor"),
            "nonce_bitmap_index": cb.fields.get("nonce_bitmap_index"),
            "signature": cb.fields.get("signature"),
        });
        let resp = client
            .post_signed("/v1/auth/approve-single", body, ctx.verbose)
            .await?;
        persist_account(&cb.account)?;
        Ok(CommandOutput::key_value(
            vec![(
                "transaction_hash".into(),
                crate::commands::market::s(&resp, "transaction_hash"),
            )],
            resp,
        ))
    }
}

fn persist_account(account: &str) -> Result<()> {
    let mut cfg = config::load()?;
    cfg.auth.account = Some(account.to_string());
    config::save(&cfg)
}

fn import(ctx: &AppContext) -> Result<CommandOutput> {
    let creds = ctx.credentials()?;
    let mut cfg = config::load()?;
    cfg.auth.private_key = Some(creds.private_key.expose().to_string());
    cfg.auth.account = Some(creds.account.clone());
    config::save(&cfg)?;
    Ok(CommandOutput::message(&format!(
        "Imported account {} (key stored at {}, mode 0600).",
        mask_address(&creds.account),
        config::config_path()?.display()
    )))
}

async fn status(ctx: &AppContext) -> Result<CommandOutput> {
    let creds = ctx.credentials()?;
    let client = ctx.client()?;
    let (allowance_status, expiry) = allowance_status(&client, &creds.account, ctx.verbose).await?;
    let pairs = vec![
        ("account".into(), creds.account.clone()),
        ("network".into(), ctx.network.to_string()),
        ("allowance".into(), allowance_status),
        ("allowance_expiry".into(), expiry),
    ];
    Ok(CommandOutput::key_value(
        pairs,
        json!({ "account": creds.account, "network": ctx.network.to_string() }),
    ))
}

async fn login(ctx: &AppContext) -> Result<CommandOutput> {
    let account = ctx.account()?;
    let signer = ctx.optional_signer();
    let client = ctx.client()?;
    let token = session::ensure_token(
        &client,
        &account,
        ctx.network.label(),
        signer.as_ref(),
        ctx.verbose,
    )
    .await?;
    Ok(CommandOutput::message(&format!(
        "Session active for {} ({}). Token length {}.",
        mask_address(&account),
        ctx.network,
        token.len()
    )))
}

async fn approve(ctx: &AppContext, budget_usd: f64, expiry: Option<&str>) -> Result<CommandOutput> {
    if budget_usd <= 0.0 {
        return Err(RisexError::Validation("budget must be positive".into()));
    }
    let (signer, account) = ctx.signer_and_account()?;
    let client = ctx.client()?;

    let operator = client.fetch_operator_hub(ctx.verbose).await?;
    let domain = client.fetch_eip712_domain(ctx.verbose).await?;
    let (nonce_anchor, bitmap_index) = client.fetch_nonce_state(&account, ctx.verbose).await?;
    let allowance_expiry = resolve_expiry(expiry)?;
    let budget_wad = (budget_usd * 1e18_f64) as u128;

    confirm_write(
        ctx,
        &format!(
            "ApproveSingle: grant operator {} a budget of ${} (expires {}).",
            mask_address(&operator),
            budget_usd,
            allowance_expiry
        ),
    )?;

    let signature = signer.sign_permit_single(
        &domain,
        &account,
        &operator,
        budget_wad,
        allowance_expiry,
        nonce_anchor,
        bitmap_index,
    )?;
    let body = json!({
        "account": account,
        "operator": operator,
        "budget": budget_wad.to_string(),
        "allowance_expiry": allowance_expiry,
        "nonce_anchor": nonce_anchor.to_string(),
        "nonce_bitmap_index": bitmap_index,
        "signature": signature,
    });
    let resp = client
        .post_signed("/v1/auth/approve-single", body, ctx.verbose)
        .await?;
    let tx = resp
        .get("transaction_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(CommandOutput::key_value(
        vec![
            ("status".into(), "approved".into()),
            ("transaction_hash".into(), tx),
        ],
        resp,
    ))
}

async fn allowance(ctx: &AppContext) -> Result<CommandOutput> {
    let creds = ctx.credentials()?;
    let client = ctx.client()?;
    let (s, expiry) = allowance_status(&client, &creds.account, ctx.verbose).await?;
    Ok(CommandOutput::key_value(
        vec![("status".into(), s), ("allowance_expiry".into(), expiry)],
        json!({ "account": creds.account }),
    ))
}

async fn refresh(ctx: &AppContext) -> Result<CommandOutput> {
    let account = ctx.account()?;
    let signer = ctx.optional_signer();
    let client = ctx.client()?;
    session::clear(ctx.network.label(), &account)?;
    let token = session::ensure_token(
        &client,
        &account,
        ctx.network.label(),
        signer.as_ref(),
        ctx.verbose,
    )
    .await?;
    Ok(CommandOutput::message(&format!(
        "Refreshed session for {} (token length {}).",
        mask_address(&account),
        token.len()
    )))
}

fn reset(ctx: &AppContext) -> Result<CommandOutput> {
    // Clear session cache if we can resolve the account; always clear config auth.
    if let Ok(creds) = ctx.credentials() {
        let _ = session::clear(ctx.network.label(), &creds.account);
    }
    let mut cfg = config::load()?;
    cfg.auth = config::AuthConfig::default();
    config::save(&cfg)?;
    Ok(CommandOutput::message(
        "Cleared stored credentials and cached session.",
    ))
}

/// GET `/v1/auth/allowance-status?account=…` → (status, expiry-string).
async fn allowance_status(
    client: &crate::client::RestClient,
    account: &str,
    verbose: bool,
) -> Result<(String, String)> {
    let d = client
        .public_get(
            &format!("/v1/auth/allowance-status?account={account}"),
            &[],
            verbose,
        )
        .await?;
    let status = d
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let expiry = d
        .get("allowance_expiry")
        .map(|v| v.to_string())
        .unwrap_or_default();
    Ok((status, expiry))
}

/// Parse an expiry argument into an absolute unix timestamp (u32).
fn resolve_expiry(arg: Option<&str>) -> Result<u32> {
    let now = chrono::Utc::now().timestamp();
    let secs: i64 = match arg {
        None => 30 * 24 * 3600,
        Some(s) => {
            let s = s.trim();
            if let Some(n) = s.strip_suffix('d') {
                n.parse::<i64>().map_err(bad)? * 86400
            } else if let Some(n) = s.strip_suffix('h') {
                n.parse::<i64>().map_err(bad)? * 3600
            } else if let Some(n) = s.strip_suffix('s') {
                n.parse::<i64>().map_err(bad)?
            } else {
                // Absolute unix timestamp.
                return Ok(s.parse::<i64>().map_err(bad)? as u32);
            }
        }
    };
    Ok((now + secs) as u32)
}

fn bad<E: std::fmt::Display>(e: E) -> RisexError {
    RisexError::Validation(format!("bad expiry: {e}"))
}
