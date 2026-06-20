//! Public REST client for RISEx. Unwraps the `{data, request_id}` envelope and
//! maps non-2xx / error bodies to RisexError. Auth (bearer) is added in Phase 2.
use serde_json::Value;

use crate::errors::{Result, RisexError};
use crate::output;
use crate::telemetry;

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

    pub async fn public_get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        verbose: bool,
    ) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        if verbose {
            output::verbose(&format!("GET {url} {query:?}"));
        }
        let mut req = self.http.get(&url).query(query);
        for (k, v) in telemetry::client_headers() {
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(RisexError::from)?;
        let status = resp.status();
        let body = resp.text().await.map_err(RisexError::from)?;
        if verbose {
            output::verbose(&format!("status {status}"));
        }
        parse_envelope(status, &body)
    }
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
        });
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(RisexError::Auth(message));
    }
    Err(RisexError::Api(message))
}
