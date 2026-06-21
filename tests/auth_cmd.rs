use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ACCT: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

#[tokio::test]
async fn auth_approve_posts_signed_permit() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/auth/eip712-domain"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"name":"RISEx","version":"1","chain_id":"11155931","verifying_contract":"0x6DA86F486b5E6536358F5b122dBe184522CA0eE3"}})))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/system/config"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"addresses":{"operator_hub":"0x0AbF5B4CDd7B1ae4f444e4Ab5E98b341567e3402"}}})))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/nonce-state/{ACCT}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"nonce_anchor":"0","current_bitmap_index":1}})))
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/approve-single"))
        .and(body_partial_json(json!({"account": ACCT, "nonce_bitmap_index": 1})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"transaction_hash":"0xabc","success":true}})))
        .mount(&s)
        .await;

    let uri = s.uri();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("risex")
            .unwrap()
            .args([
                "--api-url", &uri, "--private-key", KEY, "-y", "auth", "approve", "--budget",
                "1000",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("0xabc"));
    })
    .await
    .unwrap();
}
