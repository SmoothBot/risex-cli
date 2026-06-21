//! End-to-end JWT trading flow against a mock API: buy -> positions -> cancel.
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ACCT: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

async fn mount(s: &MockServer) {
    Mock::given(method("GET")).and(path("/v1/auth/eip712-domain"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"name":"RISEx","version":"1","chain_id":"11155931","verifying_contract":"0x6DA86F486b5E6536358F5b122dBe184522CA0eE3"}}))).mount(s).await;
    Mock::given(method("GET")).and(path_regex(r"/v1/auth/nonce.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"nonce":"0x23c6560f9a08ad3e2fab7b75ca6c36417c3242799b241f7706bf0e7f15c075a7"}}))).mount(s).await;
    Mock::given(method("POST")).and(path("/v1/auth/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"access_token":"tok","refresh_token":"r","expires_in":900}}))).mount(s).await;
    Mock::given(method("GET")).and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"markets":[
            {"market_id":"1","display_name":"BTC/USDC","visible":true,"available":true,"config":{"step_size":"0.000001","step_price":"0.1"}}
        ]}}))).mount(s).await;
    Mock::given(method("POST")).and(path("/v1/orders/place"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"order_id":"12345-100-0","tx_hash":"0xdead","sc_order_id":"12345"}}))).mount(s).await;
    Mock::given(method("GET")).and(path("/v1/positions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"positions":[
            {"market_id":"1","side":0,"size":"0.01","entry_price":"64000","mark_price":"64010","unrealized_pnl":"0.1","leverage":"5"}
        ]}}))).mount(s).await;
    Mock::given(method("POST")).and(path("/v1/orders/cancel"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"success":true,"tx_hash":"0xbeef"}}))).mount(s).await;
}

fn risex(uri: &str, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut a = vec!["--api-url", uri, "-n", "testnet", "--private-key", KEY, "-y"];
    a.extend_from_slice(args);
    Command::cargo_bin("risex").unwrap().args(a).assert()
}

#[tokio::test]
async fn buy_then_positions_then_cancel() {
    let s = MockServer::start().await;
    mount(&s).await;
    let uri = s.uri();

    tokio::task::spawn_blocking(move || {
        // 1) market buy opens a long
        risex(&uri, &["order", "buy", "btc", "0.01", "--type", "market"])
            .success()
            .stdout(predicate::str::contains("12345-100-0"));

        // 2) positions shows the open long
        risex(&uri, &["positions"])
            .success()
            .stdout(predicate::str::contains("long"));

        // 3) cancel a resting order
        risex(&uri, &["order", "cancel", "btc", "0x000000380000000900000000000000640000000000000000"])
            .success()
            .stdout(predicate::str::contains("0xbeef"));
        let _ = ACCT; // account is derived from KEY; pinned by other suites
    })
    .await
    .unwrap();
}
