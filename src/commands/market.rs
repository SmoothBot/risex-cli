//! Public market-data commands. All read-only; no auth.
use serde_json::Value;

use crate::client::RestClient;
use crate::errors::{Result, RisexError};
use crate::output::CommandOutput;

pub enum MarketCommand {
    Markets {
        market: Option<String>,
    },
    Ticker {
        market: String,
    },
    Orderbook {
        market: String,
        depth: u32,
        aggregate: Option<f64>,
        no_agg: bool,
        amount: bool,
    },
    Trades {
        market: String,
        limit: u32,
    },
    Candles {
        market: String,
        resolution: String,
        from: Option<String>,
        to: Option<String>,
    },
    Funding {
        market: String,
    },
    System,
}

pub async fn execute(
    cmd: &MarketCommand,
    client: &RestClient,
    verbose: bool,
) -> Result<CommandOutput> {
    match cmd {
        MarketCommand::Markets { market } => markets(client, market.as_deref(), verbose).await,
        MarketCommand::Ticker { market } => ticker(client, market, verbose).await,
        MarketCommand::Orderbook {
            market,
            depth,
            aggregate,
            no_agg,
            amount,
        } => orderbook(client, market, *depth, *aggregate, *no_agg, *amount, verbose).await,
        MarketCommand::Trades { market, limit } => trades(client, market, *limit, verbose).await,
        MarketCommand::Candles {
            market,
            resolution,
            from,
            to,
        } => {
            candles(
                client,
                market,
                resolution,
                from.as_deref(),
                to.as_deref(),
                verbose,
            )
            .await
        }
        MarketCommand::Funding { market } => funding(client, market, verbose).await,
        MarketCommand::System => system(client, verbose).await,
    }
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .map(|x| match x {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

/// Map a few common full asset names / synonyms to their base ticker so
/// `bitcoin` resolves like `btc`.
fn canonical_ticker(input: &str) -> &str {
    match input {
        "bitcoin" | "xbt" => "btc",
        "ether" | "ethereum" => "eth",
        "solana" => "sol",
        "dogecoin" => "doge",
        "binance" | "binancecoin" => "bnb",
        other => other,
    }
}

/// Does this market match a user-supplied identifier? Accepts the numeric
/// market id, the display name (`BTC/USDC`), the compacted name (`btcusdc`),
/// or the base ticker (`btc` / `bitcoin`). Case-insensitive.
fn market_matches(m: &Value, needle: &str) -> bool {
    let needle = needle.trim().to_ascii_lowercase();
    let needle = canonical_ticker(&needle);
    let id = s(m, "market_id").to_ascii_lowercase();
    let name = s(m, "display_name").to_ascii_lowercase(); // e.g. "btc/usdc"
    let base = name.split('/').next().unwrap_or("").to_string();
    let compact = name.replace('/', "");
    needle == id || needle == name || needle == base || needle == compact
}

/// Resolve a user-supplied market identifier to its numeric market id.
/// Pure numeric input is passed through; otherwise `/v1/markets` is consulted.
async fn resolve_market_id(client: &RestClient, input: &str, verbose: bool) -> Result<String> {
    let trimmed = input.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Ok(trimmed.to_string());
    }
    let data = client.public_get("/v1/markets", &[], verbose).await?;
    let empty = vec![];
    let list = data
        .get("markets")
        .and_then(|m| m.as_array())
        .unwrap_or(&empty);
    if let Some(m) = list.iter().find(|m| market_matches(m, trimmed)) {
        return Ok(s(m, "market_id"));
    }
    let available: Vec<String> = list
        .iter()
        .map(|m| s(m, "display_name"))
        .filter(|n| !n.is_empty())
        .take(12)
        .collect();
    Err(RisexError::Validation(format!(
        "unknown market '{input}'. Available: {}",
        available.join(", ")
    )))
}

/// Resolve a user-supplied identifier to the full market JSON object (so the
/// caller can read its tick / config). Always consults `/v1/markets`.
async fn resolve_market(client: &RestClient, input: &str, verbose: bool) -> Result<Value> {
    let data = client.public_get("/v1/markets", &[], verbose).await?;
    let empty = vec![];
    let list = data
        .get("markets")
        .and_then(|m| m.as_array())
        .unwrap_or(&empty);
    if let Some(m) = list.iter().find(|m| market_matches(m, input)) {
        return Ok((*m).clone());
    }
    let available: Vec<String> = list
        .iter()
        .map(|m| s(m, "display_name"))
        .filter(|n| !n.is_empty())
        .take(12)
        .collect();
    Err(RisexError::Validation(format!(
        "unknown market '{input}'. Available: {}",
        available.join(", ")
    )))
}

/// Default aggregation bucket for a market, derived from its price tick.
/// ~100× the tick: BTC (tick 0.1) → 10, ETH (0.01) → 1, etc.
fn default_agg(step_price: f64) -> f64 {
    step_price * 100.0
}

async fn markets(
    client: &RestClient,
    filter: Option<&str>,
    verbose: bool,
) -> Result<CommandOutput> {
    let data = client.public_get("/v1/markets", &[], verbose).await?;
    let empty = vec![];
    let list = data
        .get("markets")
        .and_then(|m| m.as_array())
        .unwrap_or(&empty);
    let headers = vec![
        "ID".into(),
        "Market".into(),
        "Last".into(),
        "Mark".into(),
        "Index".into(),
    ];
    let rows = list
        .iter()
        .filter(|m| match filter {
            Some(f) => market_matches(m, f),
            None => true,
        })
        .map(|m| {
            vec![
                s(m, "market_id"),
                s(m, "display_name"),
                fmt_price_to_tick(m, "last_price"),
                fmt_price_to_tick(m, "mark_price"),
                fmt_price_to_tick(m, "index_price"),
            ]
        })
        .collect();
    Ok(CommandOutput::new(data, headers, rows))
}

async fn ticker(client: &RestClient, market: &str, verbose: bool) -> Result<CommandOutput> {
    // Ticker is derived from /v1/markets to avoid a second endpoint contract.
    let data = client.public_get("/v1/markets", &[], verbose).await?;
    let empty = vec![];
    let found = data
        .get("markets")
        .and_then(|m| m.as_array())
        .unwrap_or(&empty)
        .iter()
        .find(|m| market_matches(m, market))
        .cloned()
        .unwrap_or(Value::Null);
    let pairs = vec![
        ("market".into(), s(&found, "display_name")),
        ("last_price".into(), fmt_price_to_tick(&found, "last_price")),
        ("mark_price".into(), fmt_price_to_tick(&found, "mark_price")),
        (
            "index_price".into(),
            fmt_price_to_tick(&found, "index_price"),
        ),
        ("change_24h".into(), s(&found, "change_24h")),
        (
            "current_funding_rate".into(),
            s(&found, "current_funding_rate"),
        ),
    ];
    Ok(CommandOutput::key_value(pairs, found))
}

/// A single (possibly aggregated) order-book level.
struct Level {
    price: f64,
    qty: f64,
    notional: f64,
}

/// Parse a `[{price, quantity}]` JSON array into `(price, qty)` pairs.
fn parse_pairs(arr: &[Value]) -> Vec<(f64, f64)> {
    arr.iter()
        .filter_map(|l| {
            let p = s(l, "price").parse::<f64>().ok()?;
            let q = s(l, "quantity").parse::<f64>().ok()?;
            Some((p, q))
        })
        .collect()
}

/// Raw (un-aggregated) levels, best price first (bids descending, asks ascending).
fn raw_levels(pairs: &[(f64, f64)], is_bid: bool) -> Vec<Level> {
    let mut out: Vec<Level> = pairs
        .iter()
        .filter(|(_, q)| *q > 0.0)
        .map(|&(price, qty)| Level {
            price,
            qty,
            notional: price * qty,
        })
        .collect();
    out.sort_by(|a, b| {
        if is_bid {
            b.price.partial_cmp(&a.price).unwrap()
        } else {
            a.price.partial_cmp(&b.price).unwrap()
        }
    });
    out
}

/// Aggregate levels into price buckets of `tick`. Bids floor to the bucket,
/// asks ceil, so a bucket never crosses the mid. Best price first.
fn aggregate(pairs: &[(f64, f64)], tick: f64, is_bid: bool) -> Vec<Level> {
    use std::collections::BTreeMap;
    let mut buckets: BTreeMap<i64, (f64, f64)> = BTreeMap::new();
    for &(price, qty) in pairs {
        if qty <= 0.0 {
            continue;
        }
        let idx = if is_bid {
            (price / tick).floor() as i64
        } else {
            (price / tick).ceil() as i64
        };
        let e = buckets.entry(idx).or_insert((0.0, 0.0));
        e.0 += qty;
        e.1 += price * qty;
    }
    // BTreeMap iterates ascending by idx (= ascending price) → best-first for asks.
    let mut out: Vec<Level> = buckets
        .into_iter()
        .map(|(idx, (qty, notional))| Level {
            price: idx as f64 * tick,
            qty,
            notional,
        })
        .collect();
    if is_bid {
        out.reverse(); // descending price → best bid first
    }
    out
}

/// Format a price/amount: up to 10 decimals, trailing zeros trimmed.
fn fmt_trim(x: f64) -> String {
    let s = format!("{x:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Format a notional (USD) value with two decimals.
fn fmt_usd(x: f64) -> String {
    format!("{x:.2}")
}

/// Number of fractional digits implied by a price tick string,
/// e.g. "0.1" → 1, "0.001" → 3, "10" → 0.
fn tick_decimals(step_price: &str) -> usize {
    match step_price.split_once('.') {
        Some((_, frac)) => frac.trim_end_matches('0').len(),
        None => 0,
    }
}

/// Format a market's price field (`last_price`/`mark_price`/`index_price`) to
/// the precision of that market's tick, so we don't render 18-decimal noise.
fn fmt_price_to_tick(m: &Value, value_key: &str) -> String {
    let raw = s(m, value_key);
    let step = m
        .get("config")
        .and_then(|c| c.get("step_price"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if step.is_empty() {
        return raw;
    }
    match raw.parse::<f64>() {
        Ok(v) => {
            let dec = tick_decimals(step);
            format!("{v:.dec$}")
        }
        Err(_) => raw,
    }
}

/// A level with running totals accumulated outward from the best price.
struct CumLevel {
    price: f64,
    cum_qty: f64,
    cum_notional: f64,
}

/// Accumulate running qty/notional totals over `levels` (which must be ordered
/// best price first), producing cumulative depth at each level.
fn cumulate(levels: &[Level]) -> Vec<CumLevel> {
    let mut cum_qty = 0.0;
    let mut cum_notional = 0.0;
    levels
        .iter()
        .map(|l| {
            cum_qty += l.qty;
            cum_notional += l.notional;
            CumLevel {
                price: l.price,
                cum_qty,
                cum_notional,
            }
        })
        .collect()
}

/// Spread between best bid and best ask, in basis points of the mid price.
fn spread_bps(best_bid: f64, best_ask: f64) -> f64 {
    let mid = (best_bid + best_ask) / 2.0;
    if mid <= 0.0 {
        return 0.0;
    }
    (best_ask - best_bid) / mid * 10_000.0
}

async fn orderbook(
    client: &RestClient,
    market: &str,
    depth: u32,
    aggregate_tick: Option<f64>,
    no_agg: bool,
    amount: bool,
    verbose: bool,
) -> Result<CommandOutput> {
    let m = resolve_market(client, market, verbose).await?;
    let market_id = s(&m, "market_id");
    let step_price = m
        .get("config")
        .and_then(|c| c.get("step_price"))
        .and_then(|v| v.as_str())
        .and_then(|x| x.parse::<f64>().ok());

    // Effective aggregation: --no-agg disables it; an explicit -a wins; otherwise
    // default to a per-market bucket derived from the tick.
    let aggregate_tick: Option<f64> = if no_agg {
        None
    } else {
        aggregate_tick.or_else(|| step_price.map(default_agg))
    };
    if verbose {
        crate::output::verbose(&format!("aggregation bucket: {aggregate_tick:?}"));
    }

    // When aggregating, pull a wider raw set so buckets have levels to gather.
    let raw_limit: u32 = if aggregate_tick.is_some() {
        500
    } else {
        depth
    };
    let raw_limit_s = raw_limit.to_string();
    let data = client
        .public_get(
            "/v1/orderbook",
            &[("market_id", &market_id), ("limit", &raw_limit_s)],
            verbose,
        )
        .await?;
    let empty = vec![];
    let bids = parse_pairs(data.get("bids").and_then(|b| b.as_array()).unwrap_or(&empty));
    let asks = parse_pairs(data.get("asks").and_then(|a| a.as_array()).unwrap_or(&empty));

    let (mut ask_levels, mut bid_levels) = match aggregate_tick {
        Some(tick) if tick > 0.0 => (aggregate(&asks, tick, false), aggregate(&bids, tick, true)),
        _ => (raw_levels(&asks, false), raw_levels(&bids, true)),
    };
    ask_levels.truncate(depth as usize);
    bid_levels.truncate(depth as usize);

    // Cumulative depth accumulated outward from the best price on each side.
    let ask_cum = cumulate(&ask_levels);
    let bid_cum = cumulate(&bid_levels);

    // The displayed measure for the bar (cumulative notional by default,
    // cumulative base amount with --amount).
    let measure = |c: &CumLevel| if amount { c.cum_qty } else { c.cum_notional };
    let max_measure = ask_cum
        .iter()
        .chain(bid_cum.iter())
        .map(measure)
        .fold(0.0_f64, f64::max);
    const BAR_WIDTH: usize = 24;

    let price_header = match aggregate_tick {
        Some(t) => format!("Price (agg {})", fmt_trim(t)),
        None => "Price (raw)".to_string(),
    };
    let headers = vec![
        "Side".into(),
        price_header,
        "Cum Amount".into(),
        "Cum Notional".into(),
        "Depth".into(),
    ];

    let mut rows: Vec<Vec<String>> = Vec::new();
    // Asks worst-first so the best ask sits just above the spread divider.
    for c in ask_cum.iter().rev() {
        rows.push(vec![
            "ask".into(),
            fmt_trim(c.price),
            fmt_trim(c.cum_qty),
            fmt_usd(c.cum_notional),
            depth_bar(measure(c), max_measure, BAR_WIDTH),
        ]);
    }
    // Spread divider row. Use the RAW best bid/ask (pre-aggregation) so the
    // bps reflects the true top-of-book, not the bucket width.
    let best_ask_px = asks
        .iter()
        .map(|(p, _)| *p)
        .fold(f64::INFINITY, f64::min);
    let best_bid_px = bids.iter().map(|(p, _)| *p).fold(f64::NEG_INFINITY, f64::max);
    if best_ask_px.is_finite() && best_bid_px.is_finite() {
        let bps = spread_bps(best_bid_px, best_ask_px);
        let gap = best_ask_px - best_bid_px;
        rows.push(vec![
            "spread".into(),
            fmt_trim(gap),
            format!("{bps:.2} bps"),
            String::new(),
            "─".repeat(BAR_WIDTH),
        ]);
    }
    for c in bid_cum.iter() {
        rows.push(vec![
            "bid".into(),
            fmt_trim(c.price),
            fmt_trim(c.cum_qty),
            fmt_usd(c.cum_notional),
            depth_bar(measure(c), max_measure, BAR_WIDTH),
        ]);
    }
    Ok(CommandOutput::new(data, headers, rows))
}

async fn trades(
    client: &RestClient,
    market: &str,
    limit: u32,
    verbose: bool,
) -> Result<CommandOutput> {
    let market_id = resolve_market_id(client, market, verbose).await?;
    let limit_s = limit.to_string();
    let data = client
        .public_get(
            "/v1/trades",
            &[("market_id", &market_id), ("limit", &limit_s)],
            verbose,
        )
        .await?;
    let empty = vec![];
    let list = data
        .get("trades")
        .and_then(|t| t.as_array())
        .or_else(|| data.as_array())
        .unwrap_or(&empty);
    let headers = vec!["Price".into(), "Size".into(), "Side".into(), "Time".into()];
    let rows = list
        .iter()
        .map(|t| {
            vec![
                s(t, "price"),
                s(t, "size"),
                s(t, "maker_side"),
                s(t, "timestamp"),
            ]
        })
        .collect();
    Ok(CommandOutput::new(data, headers, rows))
}

async fn candles(
    client: &RestClient,
    market: &str,
    resolution: &str,
    from: Option<&str>,
    to: Option<&str>,
    verbose: bool,
) -> Result<CommandOutput> {
    let market_id = resolve_market_id(client, market, verbose).await?;
    let mut query: Vec<(&str, &str)> = vec![("market_id", &market_id), ("resolution", resolution)];
    if let Some(f) = from {
        query.push(("from", f));
    }
    if let Some(t) = to {
        query.push(("to", t));
    }
    let data = client.public_get("/v1/candles", &query, verbose).await?;
    let empty = vec![];
    let list = data
        .get("candles")
        .and_then(|c| c.as_array())
        .or_else(|| data.as_array())
        .unwrap_or(&empty);
    let headers = vec![
        "Time".into(),
        "Open".into(),
        "High".into(),
        "Low".into(),
        "Close".into(),
        "Volume".into(),
    ];
    let rows = list
        .iter()
        .map(|c| {
            vec![
                s(c, "time"),
                s(c, "open"),
                s(c, "high"),
                s(c, "low"),
                s(c, "close"),
                s(c, "volume"),
            ]
        })
        .collect();
    Ok(CommandOutput::new(data, headers, rows))
}

async fn funding(client: &RestClient, market: &str, verbose: bool) -> Result<CommandOutput> {
    let market_id = resolve_market_id(client, market, verbose).await?;
    let data = client
        .public_get("/v1/funding-rates", &[("market_id", &market_id)], verbose)
        .await?;
    Ok(CommandOutput::new(
        data.clone(),
        vec!["Funding".into()],
        vec![vec![data.to_string()]],
    ))
}

/// Render an ASCII depth bar for `qty` relative to `max`, up to `width` cells.
/// Uses 1/8-block partials so small levels are still visible.
fn depth_bar(qty: f64, max: f64, width: usize) -> String {
    if max <= 0.0 || qty <= 0.0 || width == 0 {
        return String::new();
    }
    let units = (qty / max) * width as f64;
    let full = units.floor() as usize;
    let mut bar = "█".repeat(full.min(width));
    if full < width {
        let remainder = units - full as f64;
        // eighths: ▏▎▍▌▋▊▉
        let partials = ['▏', '▎', '▍', '▌', '▋', '▊', '▉'];
        let idx = (remainder * 8.0).round() as usize;
        if idx >= 1 {
            bar.push(partials[(idx - 1).min(partials.len() - 1)]);
        }
    }
    bar
}

async fn system(client: &RestClient, verbose: bool) -> Result<CommandOutput> {
    let data = client.public_get("/v1/system/config", &[], verbose).await?;
    let addresses = data.get("addresses").cloned().unwrap_or(Value::Null);
    let chain = data.get("chain").cloned().unwrap_or(Value::Null);
    let pairs = vec![
        ("chain".into(), s(&chain, "name")),
        ("chain_id".into(), s(&chain, "chain_id")),
        ("operator_hub".into(), s(&addresses, "operator_hub")),
        ("auth".into(), s(&addresses, "auth")),
        ("usdc".into(), s(&addresses, "usdc")),
        (
            "maintenance".into(),
            data.get("is_maintenance_mode")
                .map(|v| v.to_string())
                .unwrap_or_default(),
        ),
    ];
    Ok(CommandOutput::key_value(pairs, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_levels_sort_best_first() {
        let asks = raw_levels(&[(101.0, 1.0), (100.0, 2.0)], false);
        assert_eq!(asks[0].price, 100.0); // best (lowest) ask first
        let bids = raw_levels(&[(99.0, 1.0), (100.0, 2.0)], true);
        assert_eq!(bids[0].price, 100.0); // best (highest) bid first
    }

    #[test]
    fn raw_level_notional_is_price_times_qty() {
        let lvls = raw_levels(&[(100.0, 2.0)], false);
        assert_eq!(lvls[0].qty, 2.0);
        assert_eq!(lvls[0].notional, 200.0);
    }

    #[test]
    fn aggregate_asks_round_up_into_one_bucket() {
        // tick=1: ceil(100.4)=101, ceil(100.6)=101 → single bucket at 101.
        let out = aggregate(&[(100.4, 1.0), (100.6, 2.0)], 1.0, false);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].price, 101.0);
        assert_eq!(out[0].qty, 3.0);
        assert!((out[0].notional - (100.4 * 1.0 + 100.6 * 2.0)).abs() < 1e-9);
    }

    #[test]
    fn aggregate_bids_round_down_and_sort_descending() {
        // tick=10: floor into buckets 100 and 90, best (highest) first.
        let out = aggregate(&[(104.0, 1.0), (95.0, 2.0), (101.0, 1.0)], 10.0, true);
        assert_eq!(out[0].price, 100.0);
        assert_eq!(out[0].qty, 2.0); // 104 and 101 both floor to 100
        assert_eq!(out[1].price, 90.0);
        assert_eq!(out[1].qty, 2.0);
    }

    #[test]
    fn depth_bar_full_at_max() {
        assert_eq!(depth_bar(10.0, 10.0, 5), "█████");
    }

    #[test]
    fn depth_bar_half_fills_half_width() {
        // 50% of 10 cells = 5 full blocks, no partial.
        assert_eq!(depth_bar(5.0, 10.0, 10), "█████");
    }

    #[test]
    fn depth_bar_empty_for_zero_or_no_max() {
        assert_eq!(depth_bar(0.0, 10.0, 5), "");
        assert_eq!(depth_bar(5.0, 0.0, 5), "");
        assert_eq!(depth_bar(5.0, 10.0, 0), "");
    }

    #[test]
    fn depth_bar_small_level_still_visible() {
        // A tiny fraction should render at least a partial block, not blank.
        let bar = depth_bar(1.0, 100.0, 10);
        assert!(!bar.is_empty(), "small level should show a partial block");
    }
}
