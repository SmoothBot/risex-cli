use risex_cli::client::RestClient;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn public_get_unwraps_data_envelope() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "markets": [{ "market_id": "1" }] },
            "request_id": "abc-123"
        })))
        .mount(&server)
        .await;

    let client = RestClient::new(&server.uri()).unwrap();
    let data = client.public_get("/v1/markets", &[], false).await.unwrap();
    assert_eq!(data["markets"][0]["market_id"], "1");
}

#[tokio::test]
async fn public_get_maps_500_to_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let client = RestClient::new(&server.uri()).unwrap();
    let err = client.public_get("/v1/markets", &[], false).await.unwrap_err();
    assert_eq!(err.category(), "api");
}
