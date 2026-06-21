use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

async fn mount_auth(s: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/auth/eip712-domain"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"name":"RISEx","version":"1","chain_id":"11155931","verifying_contract":"0x6DA86F486b5E6536358F5b122dBe184522CA0eE3"}})))
        .mount(s)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"/v1/auth/nonce.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"nonce":"0x23c6560f9a08ad3e2fab7b75ca6c36417c3242799b241f7706bf0e7f15c075a7"}})))
        .mount(s)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"access_token":"tok","refresh_token":"r","expires_in":900}})))
        .mount(s)
        .await;
}

async fn mount_markets(s: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"markets":[
            {"market_id":"1","display_name":"BTC/USDC","visible":true,"available":true,"config":{"step_size":"0.000001","step_price":"0.1"}}
        ]}})))
        .mount(s)
        .await;
}

#[tokio::test]
async fn market_buy_places_jwt_order() {
    let s = MockServer::start().await;
    mount_auth(&s).await;
    mount_markets(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/orders/place"))
        .and(header("authorization", "Bearer tok"))
        .and(body_partial_json(
            json!({"market_id":1,"side":0,"order_type":0,"time_in_force":3,"size_steps":10000,"price_ticks":0}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"order_id":"12345-100-0","tx_hash":"0xdead"}})))
        .mount(&s)
        .await;

    let uri = s.uri();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("risex")
            .unwrap()
            .args([
                "--api-url", &uri, "-n", "testnet", "--private-key", KEY, "-y", "order", "buy",
                "btc", "0.01", "--type", "market",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("12345-100-0"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn limit_sell_converts_price_to_ticks() {
    let s = MockServer::start().await;
    mount_auth(&s).await;
    mount_markets(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/orders/place"))
        .and(body_partial_json(
            json!({"market_id":1,"side":1,"order_type":1,"time_in_force":0,"price_ticks":700000,"post_only":true}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"order_id":"9-9-0"}})))
        .mount(&s)
        .await;

    let uri = s.uri();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("risex")
            .unwrap()
            .args([
                "--api-url", &uri, "-n", "testnet", "--private-key", KEY, "-y", "order", "sell",
                "btc", "0.01", "--price", "70000", "--post-only",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("9-9-0"));
    })
    .await
    .unwrap();
}
