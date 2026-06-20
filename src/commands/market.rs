//! Public market-data commands. All read-only; no auth.
use serde_json::Value;

use crate::client::RestClient;
use crate::errors::Result;
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
        MarketCommand::Orderbook { market, depth } => {
            orderbook(client, market, *depth, verbose).await
        }
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
            Some(f) => s(m, "display_name").eq_ignore_ascii_case(f) || s(m, "market_id") == f,
            None => true,
        })
        .map(|m| {
            vec![
                s(m, "market_id"),
                s(m, "display_name"),
                s(m, "last_price"),
                s(m, "mark_price"),
                s(m, "index_price"),
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
        .find(|m| s(m, "display_name").eq_ignore_ascii_case(market) || s(m, "market_id") == market)
        .cloned()
        .unwrap_or(Value::Null);
    let pairs = vec![
        ("market".into(), s(&found, "display_name")),
        ("last_price".into(), s(&found, "last_price")),
        ("mark_price".into(), s(&found, "mark_price")),
        ("index_price".into(), s(&found, "index_price")),
        ("change_24h".into(), s(&found, "change_24h")),
        (
            "current_funding_rate".into(),
            s(&found, "current_funding_rate"),
        ),
    ];
    Ok(CommandOutput::key_value(pairs, found))
}

async fn orderbook(
    client: &RestClient,
    market: &str,
    depth: u32,
    verbose: bool,
) -> Result<CommandOutput> {
    let depth_s = depth.to_string();
    let data = client
        .public_get(
            "/v1/orderbook",
            &[("market_id", market), ("limit", &depth_s)],
            verbose,
        )
        .await?;
    let empty = vec![];
    let bids = data.get("bids").and_then(|b| b.as_array()).unwrap_or(&empty);
    let asks = data.get("asks").and_then(|a| a.as_array()).unwrap_or(&empty);
    let headers = vec!["Side".into(), "Price".into(), "Quantity".into()];
    let mut rows: Vec<Vec<String>> = Vec::new();
    for a in asks.iter().rev() {
        rows.push(vec!["ask".into(), s(a, "price"), s(a, "quantity")]);
    }
    for b in bids.iter() {
        rows.push(vec!["bid".into(), s(b, "price"), s(b, "quantity")]);
    }
    Ok(CommandOutput::new(data, headers, rows))
}

async fn trades(
    client: &RestClient,
    market: &str,
    limit: u32,
    verbose: bool,
) -> Result<CommandOutput> {
    let limit_s = limit.to_string();
    let data = client
        .public_get(
            "/v1/trades",
            &[("market_id", market), ("limit", &limit_s)],
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
    let mut query: Vec<(&str, &str)> = vec![("market_id", market), ("resolution", resolution)];
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
    let data = client
        .public_get("/v1/funding-rates", &[("market_id", market)], verbose)
        .await?;
    Ok(CommandOutput::new(
        data.clone(),
        vec!["Funding".into()],
        vec![vec![data.to_string()]],
    ))
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
