use std::io::{BufRead, BufReader};
use std::time::Duration;

use assert_cmd::cargo::cargo_bin;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn auth_connect_completes_login_from_callback() {
    let api = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/auth/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{
            "access_token":"tok","refresh_token":"r","expires_in":900
        }})))
        .mount(&api)
        .await;

    // Isolate HOME so config/session writes don't touch the real user dirs.
    let home = tempfile::tempdir().unwrap();
    let api_uri = api.uri();

    let mut child = std::process::Command::new(cargo_bin("risex"))
        .args(["--api-url", &api_uri, "-n", "testnet", "auth", "connect"])
        .env("RISEX_NO_BROWSER", "1")
        .env("HOME", home.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    // Read stderr until the CLI prints `BRIDGE port=<port> state=<state>`.
    let stderr = child.stderr.take().unwrap();
    let (port, state): (u16, String) = tokio::task::spawn_blocking(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 {
                panic!("CLI exited before printing BRIDGE line");
            }
            if let Some(rest) = line.trim().strip_prefix("BRIDGE ") {
                let mut port = 0u16;
                let mut state = String::new();
                for tok in rest.split_whitespace() {
                    if let Some(p) = tok.strip_prefix("port=") {
                        port = p.parse().unwrap();
                    }
                    if let Some(s) = tok.strip_prefix("state=") {
                        state = s.to_string();
                    }
                }
                return (port, state);
            }
        }
    })
    .await
    .unwrap();

    // Play the browser: POST the signed login callback.
    reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/callback"))
        .json(&json!({
            "state": state, "action": "login", "account": "0xabc",
            "nonce": "0x01", "deadline": 123, "signature": "0xsig"
        }))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .unwrap();

    let out = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Connected"));
}
