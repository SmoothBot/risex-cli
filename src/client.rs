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

    /// POST a JSON body without a bearer token (for approve/login/refresh).
    pub async fn post_signed(
        &self,
        path: &str,
        body: serde_json::Value,
        verbose: bool,
    ) -> Result<Value> {
        self.send_json(reqwest::Method::POST, path, &body, None, verbose)
            .await
    }

    /// POST a JSON body with `Authorization: Bearer <token>`.
    pub async fn post_bearer(
        &self,
        path: &str,
        body: serde_json::Value,
        token: &str,
        verbose: bool,
    ) -> Result<Value> {
        self.send_json(reqwest::Method::POST, path, &body, Some(token), verbose)
            .await
    }

    /// GET with `Authorization: Bearer <token>`.
    pub async fn get_bearer(
        &self,
        path: &str,
        query: &[(&str, &str)],
        token: &str,
        verbose: bool,
    ) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        if verbose {
            output::verbose(&format!("GET {url} {query:?} (bearer)"));
        }
        let mut req = self.http.get(&url).query(query).bearer_auth(token);
        for (k, v) in telemetry::client_headers() {
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(RisexError::from)?;
        let status = resp.status();
        let body = resp.text().await.map_err(RisexError::from)?;
        parse_envelope(status, &body)
    }

    /// Shared JSON-body sender. Mutations are NOT retried (may have applied).
    async fn send_json(
        &self,
        m: reqwest::Method,
        path: &str,
        body: &serde_json::Value,
        token: Option<&str>,
        verbose: bool,
    ) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        if verbose {
            output::verbose(&format!("{m} {url} {body}"));
        }
        let mut req = self.http.request(m, &url).json(body);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        for (k, v) in telemetry::client_headers() {
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(RisexError::from)?;
        let status = resp.status();
        let text = resp.text().await.map_err(RisexError::from)?;
        parse_envelope(status, &text)
    }

    /// Fetch the EIP-712 domain for this network.
    pub async fn fetch_eip712_domain(&self, verbose: bool) -> Result<crate::signing::Eip712Domain> {
        let d = self.public_get("/v1/auth/eip712-domain", &[], verbose).await?;
        let get = |k: &str| {
            d.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let chain_id = get("chain_id")
            .parse::<u64>()
            .map_err(|_| RisexError::Parse("bad chain_id in eip712-domain".into()))?;
        Ok(crate::signing::Eip712Domain {
            name: get("name"),
            version: get("version"),
            chain_id,
            verifying_contract: get("verifying_contract"),
        })
    }

    /// Fetch the OperatorHub address from system config.
    pub async fn fetch_operator_hub(&self, verbose: bool) -> Result<String> {
        let d = self.public_get("/v1/system/config", &[], verbose).await?;
        d.get("addresses")
            .and_then(|a| a.get("operator_hub"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| RisexError::Parse("operator_hub missing in system config".into()))
    }

    /// Fetch `(nonce_anchor, current_bitmap_index)` for an account.
    pub async fn fetch_nonce_state(&self, account: &str, verbose: bool) -> Result<(u64, u8)> {
        let d = self
            .public_get(&format!("/v1/nonce-state/{account}"), &[], verbose)
            .await?;
        let anchor = d
            .get("nonce_anchor")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .parse::<u64>()
            .unwrap_or(0);
        let idx = d
            .get("current_bitmap_index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8;
        Ok((anchor, idx))
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
