use risex_cli::bridge::Bridge;
use std::time::Duration;

#[tokio::test]
async fn await_callback_returns_posted_payload() {
    let bridge = Bridge::start().unwrap();
    let port = bridge.port();
    let state = bridge.state().to_string();

    // Play the browser: POST the signed payload after a beat.
    let client = reqwest::Client::new();
    let poster = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        client
            .post(format!("http://127.0.0.1:{port}/callback"))
            .json(&serde_json::json!({
                "state": state, "action": "login", "account": "0xabc",
                "nonce": "0x01", "deadline": 123, "signature": "0xsig"
            }))
            .send()
            .await
            .unwrap();
    });

    let cb = tokio::task::spawn_blocking(move || bridge.await_callback(Duration::from_secs(5)))
        .await
        .unwrap()
        .unwrap();
    poster.await.unwrap();

    assert_eq!(cb.action, "login");
    assert_eq!(cb.account, "0xabc");
    assert_eq!(cb.fields["signature"], "0xsig");
}

#[tokio::test]
async fn await_callback_times_out_without_post() {
    let bridge = Bridge::start().unwrap();
    let err = tokio::task::spawn_blocking(move || bridge.await_callback(Duration::from_millis(300)))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(err.category(), "network");
}
