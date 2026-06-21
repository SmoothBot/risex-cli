//! Trading commands (JWT path): place/cancel orders, positions, close, etc.
use clap::{Args, Subcommand};
use serde_json::{json, Value};

use crate::commands::helpers::confirm_write;
use crate::commands::market;
use crate::errors::{Result, RisexError};
use crate::output::CommandOutput;
use crate::session;
use crate::AppContext;

const MAX_TICKS: u64 = 16_777_215; // uint24

/// Convert a human size (e.g. 0.01 BTC) into integer `size_steps`.
pub fn size_to_steps(size: f64, step_size: f64) -> Result<u32> {
    if step_size <= 0.0 {
        return Err(RisexError::Validation(
            "market step_size is not positive".into(),
        ));
    }
    let steps = (size / step_size).round();
    if steps < 1.0 {
        return Err(RisexError::Validation(format!(
            "size {size} is below one step ({step_size})"
        )));
    }
    if steps > u32::MAX as f64 {
        return Err(RisexError::Validation("size too large".into()));
    }
    Ok(steps as u32)
}

/// Convert a human price (e.g. 64000.0) into integer `price_ticks` (uint24).
pub fn price_to_ticks(price: f64, step_price: f64) -> Result<u32> {
    if step_price <= 0.0 {
        return Err(RisexError::Validation(
            "market step_price is not positive".into(),
        ));
    }
    let ticks = (price / step_price).round();
    if ticks < 0.0 || ticks as u64 > MAX_TICKS {
        return Err(RisexError::Validation(format!(
            "price {price} out of range for tick {step_price}"
        )));
    }
    Ok(ticks as u32)
}

// ---- command surface --------------------------------------------------------

/// Shared arguments for `order buy` / `order sell`.
#[derive(Args)]
pub struct OrderArgs {
    /// Market id, ticker, or name (btc, BTC/USDC, 1).
    pub market: String,
    /// Order size in base units (e.g. 0.01).
    pub size: f64,
    /// Order type: limit (default) or market.
    #[arg(long = "type", default_value = "limit")]
    pub order_type: String,
    /// Limit price (required for limit orders).
    #[arg(long)]
    pub price: Option<f64>,
    /// Time-in-force: gtc (default), gtt, fok, ioc.
    #[arg(long, default_value = "gtc")]
    pub tif: String,
    /// Post-only (maker) order.
    #[arg(long)]
    pub post_only: bool,
    /// Reduce-only order.
    #[arg(long)]
    pub reduce_only: bool,
}

#[derive(Subcommand)]
pub enum OrderCommand {
    /// Place a buy / long order.
    Buy(OrderArgs),
    /// Place a sell / short order.
    Sell(OrderArgs),
    /// Cancel a resting order by id.
    Cancel {
        /// Market id, ticker, or name.
        market: String,
        /// Composite order id (0x…).
        order_id: String,
    },
    /// Cancel all resting orders in a market.
    CancelAll {
        /// Market id, ticker, or name.
        market: String,
    },
}

fn cfg_f64(m: &Value, key: &str) -> f64 {
    m.get("config")
        .and_then(|c| c.get(key))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn tif_code(s: &str) -> Result<u32> {
    match s.to_ascii_lowercase().as_str() {
        "gtc" => Ok(0),
        "gtt" => Ok(1),
        "fok" => Ok(2),
        "ioc" => Ok(3),
        other => Err(RisexError::Validation(format!("unknown tif '{other}'"))),
    }
}

pub async fn execute_order(cmd: &OrderCommand, ctx: &AppContext) -> Result<CommandOutput> {
    match cmd {
        OrderCommand::Buy(a) => place(ctx, a, 0).await,
        OrderCommand::Sell(a) => place(ctx, a, 1).await,
        OrderCommand::Cancel { market, order_id } => cancel(ctx, market, order_id).await,
        OrderCommand::CancelAll { market } => cancel_all(ctx, market).await,
    }
}

async fn place(ctx: &AppContext, args: &OrderArgs, side: u32) -> Result<CommandOutput> {
    let client = ctx.client()?;
    let m = market::resolve_market(&client, &args.market, ctx.verbose).await?;
    let market_id: u32 = market::s(&m, "market_id")
        .parse()
        .map_err(|_| RisexError::Validation("bad market id".into()))?;
    let name = market::s(&m, "display_name");
    let step_size = cfg_f64(&m, "step_size");
    let step_price = cfg_f64(&m, "step_price");

    let size_steps = size_to_steps(args.size, step_size)?;
    let is_market = args.order_type.eq_ignore_ascii_case("market");
    let (order_type, price_ticks, tif) = if is_market {
        (0u32, 0u32, 3u32) // Market => IOC, price 0
    } else {
        let price = args
            .price
            .ok_or_else(|| RisexError::Validation("limit order requires --price".into()))?;
        (1u32, price_to_ticks(price, step_price)?, tif_code(&args.tif)?)
    };

    let side_word = if side == 0 { "BUY" } else { "SELL" };
    let price_word = if is_market {
        "market".to_string()
    } else {
        format!("{}", args.price.unwrap_or(0.0))
    };
    confirm_write(
        ctx,
        &format!("Order: {side_word} {} {name} @ {price_word}", args.size),
    )?;

    let (signer, account) = ctx.signer_and_account()?;
    let token =
        session::ensure_token(&client, &signer, &account, ctx.network.label(), ctx.verbose).await?;
    let body = json!({
        "market_id": market_id,
        "size_steps": size_steps,
        "price_ticks": price_ticks,
        "side": side,
        "post_only": args.post_only,
        "reduce_only": args.reduce_only,
        "stp_mode": 0,
        "order_type": order_type,
        "time_in_force": tif,
        "builder_id": 0,
        "client_order_id": 0,
        "ttl_units": 0,
    });
    let resp = client
        .post_bearer("/v1/orders/place", body, &token, ctx.verbose)
        .await?;
    Ok(CommandOutput::key_value(
        vec![
            ("order_id".into(), market::s(&resp, "order_id")),
            ("tx_hash".into(), market::s(&resp, "tx_hash")),
            ("sc_order_id".into(), market::s(&resp, "sc_order_id")),
            ("filled_percent".into(), market::s(&resp, "filled_percent")),
        ],
        resp,
    ))
}

async fn cancel(ctx: &AppContext, market_in: &str, order_id: &str) -> Result<CommandOutput> {
    let client = ctx.client()?;
    let m = market::resolve_market(&client, market_in, ctx.verbose).await?;
    let market_id: u32 = market::s(&m, "market_id")
        .parse()
        .map_err(|_| RisexError::Validation("bad market id".into()))?;
    confirm_write(ctx, &format!("Cancel order {order_id} in {market_in}"))?;
    let (signer, account) = ctx.signer_and_account()?;
    let token =
        session::ensure_token(&client, &signer, &account, ctx.network.label(), ctx.verbose).await?;
    let resp = client
        .post_bearer(
            "/v1/orders/cancel",
            json!({ "market_id": market_id, "order_id": order_id }),
            &token,
            ctx.verbose,
        )
        .await?;
    Ok(CommandOutput::key_value(
        vec![
            ("success".into(), market::s(&resp, "success")),
            ("tx_hash".into(), market::s(&resp, "tx_hash")),
        ],
        resp,
    ))
}

async fn cancel_all(ctx: &AppContext, market_in: &str) -> Result<CommandOutput> {
    let client = ctx.client()?;
    let m = market::resolve_market(&client, market_in, ctx.verbose).await?;
    let market_id: u32 = market::s(&m, "market_id")
        .parse()
        .map_err(|_| RisexError::Validation("bad market id".into()))?;
    confirm_write(ctx, &format!("Cancel ALL orders in {market_in}"))?;
    let (signer, account) = ctx.signer_and_account()?;
    let token =
        session::ensure_token(&client, &signer, &account, ctx.network.label(), ctx.verbose).await?;
    let resp = client
        .post_bearer(
            "/v1/orders/cancel-all",
            json!({ "market_id": market_id }),
            &token,
            ctx.verbose,
        )
        .await?;
    Ok(CommandOutput::key_value(
        vec![
            ("success".into(), market::s(&resp, "success")),
            ("tx_hash".into(), market::s(&resp, "tx_hash")),
        ],
        resp,
    ))
}

// ---- account reads + position management ------------------------------------

pub async fn positions(ctx: &AppContext, market_filter: Option<&str>) -> Result<CommandOutput> {
    let creds = ctx.credentials()?;
    let client = ctx.client()?;
    let data = client
        .public_get(
            &format!("/v1/positions?account={}", creds.account),
            &[],
            ctx.verbose,
        )
        .await?;
    let empty = vec![];
    let list = data
        .get("positions")
        .and_then(|p| p.as_array())
        .or_else(|| data.as_array())
        .unwrap_or(&empty);
    let headers = vec![
        "Market".into(),
        "Side".into(),
        "Size".into(),
        "Entry".into(),
        "Mark".into(),
        "uPnL".into(),
        "Liq".into(),
        "Lev".into(),
    ];
    let rows = list
        .iter()
        .filter(|p| match market_filter {
            Some(f) => {
                market::s(p, "market_id") == f
                    || market::s(p, "display_name").eq_ignore_ascii_case(f)
            }
            None => true,
        })
        .map(|p| {
            vec![
                market::s(p, "market_id"),
                side_label(&market::s(p, "side")),
                market::s(p, "size"),
                market::s(p, "entry_price"),
                market::s(p, "mark_price"),
                market::s(p, "unrealized_pnl"),
                market::s(p, "liquidation_price"),
                market::s(p, "leverage"),
            ]
        })
        .collect();
    Ok(CommandOutput::new(data, headers, rows))
}

pub async fn balance(ctx: &AppContext) -> Result<CommandOutput> {
    let creds = ctx.credentials()?;
    let client = ctx.client()?;
    let data = client
        .public_get(
            &format!("/v1/account/balance?account={}", creds.account),
            &[],
            ctx.verbose,
        )
        .await?;
    let pairs = match data.as_object() {
        Some(map) => map
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    },
                )
            })
            .collect(),
        None => vec![("balance".into(), data.to_string())],
    };
    Ok(CommandOutput::key_value(pairs, data))
}

pub async fn close(ctx: &AppContext, market_in: &str) -> Result<CommandOutput> {
    let client = ctx.client()?;
    let m = market::resolve_market(&client, market_in, ctx.verbose).await?;
    let market_id: u32 = market::s(&m, "market_id")
        .parse()
        .map_err(|_| RisexError::Validation("bad market id".into()))?;
    let step_size = cfg_f64(&m, "step_size");

    // Find the open position for this market.
    let creds = ctx.credentials()?;
    let pos_data = client
        .public_get(
            &format!("/v1/positions?account={}", creds.account),
            &[],
            ctx.verbose,
        )
        .await?;
    let empty = vec![];
    let list = pos_data
        .get("positions")
        .and_then(|p| p.as_array())
        .or_else(|| pos_data.as_array())
        .unwrap_or(&empty);
    let pos = list
        .iter()
        .find(|p| market::s(p, "market_id") == market_id.to_string())
        .ok_or_else(|| RisexError::Validation(format!("no open position in {market_in}")))?;

    let size: f64 = market::s(pos, "size").parse::<f64>().unwrap_or(0.0).abs();
    if size <= 0.0 {
        return Err(RisexError::Validation("position size is zero".into()));
    }
    let size_steps = size_to_steps(size, step_size)?;
    // Close with the opposite side: long(0) -> sell(1), short(1) -> buy(0).
    let pos_side = market::s(pos, "side");
    let close_side: u32 = if pos_side == "0" { 1 } else { 0 };

    confirm_write(
        ctx,
        &format!("Close position in {market_in} ({size} @ market, reduce-only)"),
    )?;
    let (signer, account) = ctx.signer_and_account()?;
    let token =
        session::ensure_token(&client, &signer, &account, ctx.network.label(), ctx.verbose).await?;
    let body = json!({
        "market_id": market_id, "size_steps": size_steps, "price_ticks": 0,
        "side": close_side, "post_only": false, "reduce_only": true, "stp_mode": 0,
        "order_type": 0, "time_in_force": 3, "builder_id": 0, "client_order_id": 0, "ttl_units": 0,
    });
    let resp = client
        .post_bearer("/v1/orders/place", body, &token, ctx.verbose)
        .await?;
    Ok(CommandOutput::key_value(
        vec![
            ("order_id".into(), market::s(&resp, "order_id")),
            ("tx_hash".into(), market::s(&resp, "tx_hash")),
        ],
        resp,
    ))
}

pub async fn leverage(ctx: &AppContext, market_in: &str, lev: f64) -> Result<CommandOutput> {
    let client = ctx.client()?;
    let m = market::resolve_market(&client, market_in, ctx.verbose).await?;
    let market_id: u32 = market::s(&m, "market_id")
        .parse()
        .map_err(|_| RisexError::Validation("bad market id".into()))?;
    confirm_write(ctx, &format!("Set leverage {lev}x on {market_in}"))?;
    let (signer, account) = ctx.signer_and_account()?;
    let token =
        session::ensure_token(&client, &signer, &account, ctx.network.label(), ctx.verbose).await?;
    let resp = client
        .post_bearer(
            "/v1/account/leverage",
            json!({ "market_id": market_id, "leverage": format!("{lev}") }),
            &token,
            ctx.verbose,
        )
        .await?;
    Ok(CommandOutput::key_value(
        vec![(
            "transaction_hash".into(),
            market::s(&resp, "transaction_hash"),
        )],
        resp,
    ))
}

pub async fn margin(ctx: &AppContext, market_in: &str, mode: &str) -> Result<CommandOutput> {
    let margin_mode: u32 = match mode.to_ascii_lowercase().as_str() {
        "cross" => 0,
        "isolated" => 1,
        other => return Err(RisexError::Validation(format!("unknown margin mode '{other}'"))),
    };
    let client = ctx.client()?;
    let m = market::resolve_market(&client, market_in, ctx.verbose).await?;
    let market_id: u32 = market::s(&m, "market_id")
        .parse()
        .map_err(|_| RisexError::Validation("bad market id".into()))?;
    confirm_write(ctx, &format!("Set {mode} margin on {market_in}"))?;
    let (signer, account) = ctx.signer_and_account()?;
    let token =
        session::ensure_token(&client, &signer, &account, ctx.network.label(), ctx.verbose).await?;
    let resp = client
        .post_bearer(
            "/v1/account/margin-mode",
            json!({ "market_id": market_id, "margin_mode": margin_mode }),
            &token,
            ctx.verbose,
        )
        .await?;
    Ok(CommandOutput::key_value(
        vec![(
            "transaction_hash".into(),
            market::s(&resp, "transaction_hash"),
        )],
        resp,
    ))
}

fn side_label(raw: &str) -> String {
    match raw.trim() {
        "0" => "long".to_string(),
        "1" => "short".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_steps_round_to_nearest() {
        assert_eq!(size_to_steps(0.01, 0.000001).unwrap(), 10_000);
        assert_eq!(size_to_steps(0.1, 0.001).unwrap(), 100);
    }

    #[test]
    fn price_ticks_round_to_nearest() {
        assert_eq!(price_to_ticks(64000.0, 0.1).unwrap(), 640_000);
        assert_eq!(price_to_ticks(1700.55, 0.01).unwrap(), 170_055);
    }

    #[test]
    fn price_ticks_overflow_errors() {
        assert!(price_to_ticks(2_000_000.0, 0.1).is_err()); // > uint24
    }

    #[test]
    fn zero_size_errors() {
        assert!(size_to_steps(0.0, 0.001).is_err());
    }
}
