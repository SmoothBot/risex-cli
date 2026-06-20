use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn markets_command_renders_json_rows() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "markets": [{
                "market_id": "1",
                "display_name": "BTC/USDC",
                "last_price": "63113.1",
                "mark_price": "63268.7",
                "index_price": "63344.5"
            }]},
            "request_id": "r1"
        })))
        .mount(&server)
        .await;

    let uri = server.uri();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("risex")
            .unwrap()
            .args(["--api-url", &uri, "-o", "json", "markets"])
            .assert()
            .success()
            .stdout(predicate::str::contains("BTC/USDC"));
    })
    .await
    .unwrap();
}
