//! Public REST client for RISEx. Unwraps the `{data, request_id}` envelope and
//! maps non-2xx / error bodies to RisexError. Auth (bearer) is added in Phase 2.
use std::time::Duration;

use serde_json::Value;

use crate::errors::{Result, RisexError};
use crate::output;
use crate::telemetry;

/// Max retry attempts for idempotent requests on transient/5xx failures.
const MAX_RETRIES: u32 = 3;
/// Base backoff in milliseconds (doubles each attempt: 500ms, 1s, 2s).
const INITIAL_BACKOFF_MS: u64 = 500;

pub struct RestClient {
    http: reqwest::Client,
    base_url: String,
}

impl RestClient {
    pub fn new(base_url: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(format!("{}/{}", telemetry::CLIENT_NAME, telemetry::version()))
            .build()
            .map_err(|e| RisexError::Network(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Public GET. Idempotent, so it retries transient network errors and 5xx
    /// responses up to `MAX_RETRIES` with exponential backoff.
    pub async fn public_get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        verbose: bool,
    ) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let mut attempt: u32 = 0;
        loop {
            if verbose {
                output::verbose(&format!("GET {url} {query:?}"));
            }
            let mut req = self.http.get(&url).query(query);
            for (k, v) in telemetry::client_headers() {
                req = req.header(k, v);
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.map_err(RisexError::from)?;
                    if status.is_server_error() && attempt < MAX_RETRIES {
                        attempt += 1;
                        backoff(attempt, verbose, &format!("server error {status}")).await;
                        continue;
                    }
                    if verbose {
                        output::verbose(&format!("status {status}"));
                    }
                    return parse_envelope(status, &body);
                }
                Err(e) => {
                    let transient = e.is_timeout() || e.is_connect();
                    if transient && attempt < MAX_RETRIES {
                        attempt += 1;
                        backoff(attempt, verbose, &e.to_string()).await;
                        continue;
                    }
                    return Err(RisexError::from(e));
                }
            }
        }
    }
}

/// Sleep with exponential backoff before retry `attempt` (1-based).
async fn backoff(attempt: u32, verbose: bool, reason: &str) {
    let ms = INITIAL_BACKOFF_MS * 2u64.pow(attempt - 1);
    if verbose {
        output::verbose(&format!(
            "{reason}; retry {attempt}/{MAX_RETRIES} after {ms}ms"
        ));
    }
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// Unwrap `{data, request_id}` on success; map errors otherwise.
fn parse_envelope(status: reqwest::StatusCode, body: &str) -> Result<Value> {
    if status.is_success() {
        let json: Value = serde_json::from_str(body).map_err(|_| {
            RisexError::Parse(format!("non-JSON response (status {status}): {body}"))
        })?;
        if let Some(data) = json.get("data") {
            return Ok(data.clone());
        }
        // Some endpoints may return a bare body; pass it through.
        return Ok(json);
    }

    // Error path: prefer an explicit message field, else fall back to the raw
    // body or the status line. Always categorize by HTTP status.
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|j| {
            j.get("error")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
                .or_else(|| j.get("message").and_then(|m| m.as_str()).map(|s| s.to_string()))
        })
        .unwrap_or_else(|| {
            if body.trim().is_empty() {
                format!("HTTP {status}")
            } else {
                body.trim().to_string()
            }
        });

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(RisexError::RateLimit {
            message: format!("rate limited: {message}"),
            retryable: true,
            suggestion: Some(
                "Back off a few seconds before retrying. The RISEx REST API allows \
                 roughly 500 requests per 10 seconds."
                    .into(),
            ),
            docs_url: None,
        });
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(RisexError::Auth(message));
    }
    Err(RisexError::Api(message))
}
