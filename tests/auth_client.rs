use risex_cli::client::RestClient;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn fetch_domain_parses_fields() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/auth/eip712-domain"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"name":"RISEx","version":"1","chain_id":"11155931","verifying_contract":"0x6DA86F486b5E6536358F5b122dBe184522CA0eE3"}
        })))
        .mount(&s)
        .await;
    let c = RestClient::new(&s.uri()).unwrap();
    let d = c.fetch_eip712_domain(false).await.unwrap();
    assert_eq!(d.name, "RISEx");
    assert_eq!(d.chain_id, 11155931);
}

#[tokio::test]
async fn fetch_operator_hub_and_nonce_state() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/system/config"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"addresses": {"operator_hub": "0xOP"}}
        })))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/nonce-state/0xabc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"nonce_anchor": "5", "current_bitmap_index": 3}
        })))
        .mount(&s)
        .await;
    let c = RestClient::new(&s.uri()).unwrap();
    assert_eq!(c.fetch_operator_hub(false).await.unwrap(), "0xOP");
    assert_eq!(c.fetch_nonce_state("0xabc", false).await.unwrap(), (5, 3));
}

#[tokio::test]
async fn post_bearer_sends_authorization_header() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/orders/place"))
        .and(header("authorization", "Bearer tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"order_id":"1-2-0"}})))
        .mount(&s)
        .await;
    let c = RestClient::new(&s.uri()).unwrap();
    let out = c
        .post_bearer("/v1/orders/place", json!({"market_id":1}), "tok", false)
        .await
        .unwrap();
    assert_eq!(out["order_id"], "1-2-0");
}
