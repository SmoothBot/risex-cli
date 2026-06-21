//! End-to-end integration tests: drive the real `risex` binary against a
//! hermetic wiremock server via `--api-url`. Deterministic, no live network.
use assert_cmd::Command;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Result of running the binary once.
struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// Run the `risex` binary with `--api-url <uri>` plus `args`, hermetically.
/// `assert_cmd` is synchronous, so we run it on a blocking thread.
async fn run(uri: &str, args: &[&str]) -> Run {
    let uri = uri.to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let mut cmd = Command::cargo_bin("risex").unwrap();
        cmd.arg("--api-url").arg(&uri);
        cmd.args(&args);
        let out = cmd.output().unwrap();
        Run {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    })
    .await
    .unwrap()
}

/// Like `run`, but sets environment variables and does NOT pass `--api-url`.
async fn run_with_env(envs: &[(&str, &str)], args: &[&str]) -> Run {
    let envs: Vec<(String, String)> = envs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let mut cmd = Command::cargo_bin("risex").unwrap();
        cmd.env_clear();
        // Keep PATH/HOME so the process and dirs crate behave.
        if let Ok(p) = std::env::var("PATH") {
            cmd.env("PATH", p);
        }
        if let Ok(h) = std::env::var("HOME") {
            cmd.env("HOME", h);
        }
        for (k, v) in &envs {
            cmd.env(k, v);
        }
        cmd.args(&args);
        let out = cmd.output().unwrap();
        Run {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    })
    .await
    .unwrap()
}

/// A market fixture with configurable fields.
#[allow(clippy::too_many_arguments)]
fn market(
    id: &str,
    name: &str,
    last: &str,
    mark: &str,
    index: &str,
    vol: &str,
    oi: &str,
    step_price: &str,
) -> serde_json::Value {
    json!({
        "market_id": id,
        "display_name": name,
        "base_asset_symbol": name,
        "quote_asset_symbol": "USDC",
        "last_price": last,
        "mark_price": mark,
        "index_price": index,
        "quote_volume_24h": vol,
        "open_interest": oi,
        "change_24h": "100",
        "current_funding_rate": "0.0001",
        "visible": true,
        "available": true,
        "config": {
            "step_price": step_price,
            "step_size": "0.000001",
            "min_order_size": "0.0001",
            "max_leverage": "50"
        }
    })
}

/// Mount the full set of market-data endpoints with deterministic fixtures.
async fn mount_fixtures(server: &MockServer) {
    // Markets: BTC (high vol), ETH (mid), ONDO (zero vol), and a high-vol
    // DEPRECATED market that must never surface anywhere.
    let markets = json!({
        "data": { "markets": [
            market("1", "BTC/USDC", "64000.1", "64000.123456789012345", "64001.98765", "8000000", "12.5", "0.1"),
            market("2", "ETH/USDC", "1700.55", "1700.559999999999", "1700.91", "3000000", "500.0", "0.01"),
            market("9", "ONDO/USDC", "0", "0.336510291954", "0.336510291954", "0", "0", "0.0001"),
            market("13", "DOGE/USDC [deprecated-123]", "0", "0.0833", "0.0833", "999999999", "1.0", "0.00001"),
        ]},
        "request_id": "t-markets"
    });
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(markets))
        .mount(server)
        .await;

    let book = json!({
        "data": {
            "market_id": "1",
            "bids": [
                {"price": "63999.9", "quantity": "1.0"},
                {"price": "63999.8", "quantity": "2.0"}
            ],
            "asks": [
                {"price": "64000.1", "quantity": "1.5"},
                {"price": "64000.2", "quantity": "0.5"}
            ]
        },
        "request_id": "t-book"
    });
    Mock::given(method("GET"))
        .and(path("/v1/orderbook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(book))
        .mount(server)
        .await;

    let trades = json!({
        "data": { "market_id": "1", "trades": [
            {"time": "1782021462000000000", "maker_side": "BUY",  "price": "64000.1", "size": "0.5"},
            {"time": "1782021460000000000", "maker_side": "SELL", "price": "63999.9", "size": "0.25"}
        ]},
        "request_id": "t-trades"
    });
    Mock::given(method("GET"))
        .and(path("/v1/markets/id/1/trade-history"))
        .respond_with(ResponseTemplate::new(200).set_body_json(trades))
        .mount(server)
        .await;

    // Candles are double-wrapped: gateway envelope + a TradingView-style `data` array.
    let candles = json!({
        "data": { "data": [
            {"time": "1782018000000000000", "open": "64000", "high": "64100", "low": "63900", "close": "64050", "volume": "1.5"}
        ]},
        "request_id": "t-candles"
    });
    Mock::given(method("GET"))
        .and(path("/v1/markets/id/1/trading-view-data"))
        .respond_with(ResponseTemplate::new(200).set_body_json(candles))
        .mount(server)
        .await;

    let funding = json!({
        "data": { "market_id": "1", "records": [
            {"block_time": "1782018000000000000", "funding_rate": "0.0001", "index_price": "64000"}
        ]},
        "request_id": "t-funding"
    });
    Mock::given(method("GET"))
        .and(path("/v1/markets/id/1/funding-rate-history"))
        .respond_with(ResponseTemplate::new(200).set_body_json(funding))
        .mount(server)
        .await;

    let system = json!({
        "data": {
            "chain": { "name": "Rise Mainnet", "chain_id": "4153" },
            "addresses": { "operator_hub": "0xophub", "auth": "0xauth", "usdc": "0xusdc" },
            "is_maintenance_mode": false
        },
        "request_id": "t-system"
    });
    Mock::given(method("GET"))
        .and(path("/v1/system/config"))
        .respond_with(ResponseTemplate::new(200).set_body_json(system))
        .mount(server)
        .await;
}

// ---- markets ----------------------------------------------------------------

#[tokio::test]
async fn markets_lists_active_sorted_by_volume() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["markets"]).await;
    assert!(r.ok, "stderr: {}", r.stderr);

    // Columns present.
    assert!(r.stdout.contains("24h Vol"));
    assert!(r.stdout.contains("OI"));

    // Sorted by 24h volume desc: BTC before ETH before ONDO.
    let btc = r.stdout.find("BTC/USDC").unwrap();
    let eth = r.stdout.find("ETH/USDC").unwrap();
    let ondo = r.stdout.find("ONDO/USDC").unwrap();
    assert!(btc < eth && eth < ondo, "unexpected order:\n{}", r.stdout);
}

#[tokio::test]
async fn markets_hides_deprecated_even_with_high_volume() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["markets"]).await;
    assert!(r.ok);
    assert!(!r.stdout.contains("deprecated"), "deprecated leaked:\n{}", r.stdout);
    assert!(!r.stdout.contains("DOGE"), "DOGE leaked:\n{}", r.stdout);
}

#[tokio::test]
async fn markets_volume_uses_thousands_separators() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["markets"]).await;
    assert!(r.stdout.contains("8,000,000"), "no grouped volume:\n{}", r.stdout);
}

#[tokio::test]
async fn markets_filter_to_one() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["markets", "--market", "eth"]).await;
    assert!(r.ok);
    assert!(r.stdout.contains("ETH/USDC"));
    assert!(!r.stdout.contains("BTC/USDC"));
}

#[tokio::test]
async fn markets_json_output_is_raw_payload() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["-o", "json", "markets"]).await;
    assert!(r.ok);
    let v: serde_json::Value = serde_json::from_str(r.stdout.trim()).expect("valid JSON");
    assert_eq!(v["markets"][0]["market_id"], "1");
}

// ---- ticker -----------------------------------------------------------------

#[tokio::test]
async fn ticker_rounds_price_to_tick() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["ticker", "btc"]).await;
    assert!(r.ok, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("64000.1"), "missing rounded mark:\n{}", r.stdout);
    assert!(
        !r.stdout.contains("64000.123456"),
        "raw high-precision mark leaked:\n{}",
        r.stdout
    );
}

// ---- orderbook --------------------------------------------------------------

#[tokio::test]
async fn orderbook_default_aggregates_and_shows_spread() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["orderbook", "btc"]).await;
    assert!(r.ok, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("Price (agg 10)"), "no agg header:\n{}", r.stdout);
    assert!(r.stdout.contains("spread"), "no spread row:\n{}", r.stdout);
    assert!(r.stdout.contains("bps"), "no bps:\n{}", r.stdout);
    assert!(r.stdout.contains("Cum Notional"));
}

#[tokio::test]
async fn orderbook_no_agg_shows_raw_prices() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["orderbook", "btc", "--no-agg"]).await;
    assert!(r.ok);
    assert!(r.stdout.contains("Price (raw)"), "no raw header:\n{}", r.stdout);
    assert!(r.stdout.contains("64000.1"), "raw ask price missing:\n{}", r.stdout);
}

#[tokio::test]
async fn book_is_alias_for_orderbook() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["book", "btc"]).await;
    assert!(r.ok, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("spread"));
}

// ---- trades / candles / funding --------------------------------------------

#[tokio::test]
async fn trades_format_side_and_timestamp() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["trades", "btc", "--limit", "2"]).await;
    assert!(r.ok, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("buy"), "no buy side:\n{}", r.stdout);
    assert!(r.stdout.contains("sell"), "no sell side:\n{}", r.stdout);
    // Nanosecond timestamp must be humanized, not printed raw.
    assert!(r.stdout.contains("2026-"), "timestamp not formatted:\n{}", r.stdout);
    assert!(!r.stdout.contains("1782021462000000000"), "raw ns leaked");
}

#[tokio::test]
async fn candles_parse_nested_data_array() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["candles", "btc", "--resolution", "60"]).await;
    assert!(r.ok, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("64050"), "close missing:\n{}", r.stdout);
    assert!(r.stdout.contains("2026-"), "candle time not formatted:\n{}", r.stdout);
}

#[tokio::test]
async fn funding_shows_rate_and_percent() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["funding", "btc"]).await;
    assert!(r.ok, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("Rate %"));
    assert!(r.stdout.contains("0.0100%"), "percent not computed:\n{}", r.stdout);
}

// ---- system -----------------------------------------------------------------

#[tokio::test]
async fn system_shows_chain_and_addresses() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["system"]).await;
    assert!(r.ok);
    assert!(r.stdout.contains("4153"));
    assert!(r.stdout.contains("0xophub"));
}

// ---- market resolution ------------------------------------------------------

#[tokio::test]
async fn resolves_market_by_full_name() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    // "bitcoin" -> btc -> market id 1 -> trade-history endpoint.
    let r = run(&server.uri(), &["trades", "bitcoin", "--limit", "2"]).await;
    assert!(r.ok, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("64000.1"));
}

#[tokio::test]
async fn unknown_market_errors_with_available_list() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    let r = run(&server.uri(), &["orderbook", "pepe"]).await;
    assert!(!r.ok, "should have failed");
    let combined = format!("{}{}", r.stdout, r.stderr);
    assert!(combined.contains("unknown market 'pepe'"), "msg:\n{combined}");
    assert!(combined.contains("BTC/USDC"), "no available hint:\n{combined}");
    // Deprecated market must not be offered as a suggestion.
    assert!(!combined.contains("deprecated"));
}

// ---- error mapping ----------------------------------------------------------

#[tokio::test]
async fn server_500_maps_to_api_error_envelope_in_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    let r = run(&server.uri(), &["-o", "json", "markets"]).await;
    assert!(!r.ok);
    let v: serde_json::Value = serde_json::from_str(r.stdout.trim()).expect("error envelope JSON");
    assert_eq!(v["error"], "api");
}

#[tokio::test]
async fn server_429_maps_to_retryable_rate_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
        .mount(&server)
        .await;
    let r = run(&server.uri(), &["-o", "json", "markets"]).await;
    assert!(!r.ok);
    let v: serde_json::Value = serde_json::from_str(r.stdout.trim()).unwrap();
    assert_eq!(v["error"], "rate_limit");
    assert_eq!(v["retryable"], true);
    assert!(v["suggestion"].is_string(), "no suggestion: {v}");
}

// ---- retry + env overrides --------------------------------------------------

#[tokio::test]
async fn retries_transient_5xx_then_succeeds() {
    let server = MockServer::start().await;
    let markets = json!({ "data": { "markets": [
        market("1", "BTC/USDC", "64000.1", "64000.1", "64000.1", "1", "1", "0.1")
    ]}, "request_id": "t" });

    // wiremock prioritizes the first-mounted matching mock, so mount the 503
    // first (it serves the first 2 attempts, then exhausts) and the 200 as the
    // lower-priority fallback the 3rd attempt falls through to.
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .expect(2) // fails the test unless retries actually consume both 503s
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(markets))
        .mount(&server)
        .await;

    let r = run(&server.uri(), &["markets"]).await;
    assert!(r.ok, "should recover after retries; stderr: {}", r.stderr);
    assert!(r.stdout.contains("BTC/USDC"));
}

#[tokio::test]
async fn env_risex_api_url_is_honored() {
    let server = MockServer::start().await;
    mount_fixtures(&server).await;
    // No --api-url flag; the base URL comes from RISEX_API_URL.
    let r = run_with_env(&[("RISEX_API_URL", &server.uri())], &["markets"]).await;
    assert!(r.ok, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("BTC/USDC"));
}

#[tokio::test]
async fn server_401_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(401).set_body_string("nope"))
        .mount(&server)
        .await;
    let r = run(&server.uri(), &["-o", "json", "markets"]).await;
    assert!(!r.ok);
    let v: serde_json::Value = serde_json::from_str(r.stdout.trim()).unwrap();
    assert_eq!(v["error"], "auth");
}

// ---- basics -----------------------------------------------------------------

#[tokio::test]
async fn help_lists_book_alias_subcommands() {
    // No network needed; --help short-circuits.
    let server = MockServer::start().await;
    let r = run(&server.uri(), &["--help"]).await;
    assert!(r.ok);
    assert!(r.stdout.contains("orderbook"));
    assert!(r.stdout.contains("markets"));
}
