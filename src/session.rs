//! JWT session lifecycle: login, refresh, on-disk cache, freshness checks.
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::client::RestClient;
use crate::config;
use crate::errors::{Result, RisexError};
use crate::signing::Signer;

#[derive(Default, Serialize, Deserialize)]
struct TokenCache {
    access_token: String,
    refresh_token: String,
    access_expiry: i64, // unix seconds
    refresh_expiry: i64,
}

/// A token is usable only if more than 30s remain before expiry.
pub fn token_is_fresh(expiry_unix: i64, now_unix: i64) -> bool {
    expiry_unix - now_unix > 30
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

fn cache_path(network_label: &str, account: &str) -> Result<PathBuf> {
    Ok(config::data_dir()?.join(format!(
        "session-{network_label}-{}.json",
        account.to_lowercase()
    )))
}

fn load_cache(p: &PathBuf) -> TokenCache {
    fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_cache(p: &PathBuf, c: &TokenCache) -> Result<()> {
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    let data = serde_json::to_string(c)?;
    write_0600(p, data.as_bytes())
}

#[cfg(unix)]
fn write_0600(p: &PathBuf, data: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(p)?;
    f.write_all(data)?;
    Ok(())
}
#[cfg(not(unix))]
fn write_0600(p: &PathBuf, data: &[u8]) -> Result<()> {
    fs::write(p, data)?;
    Ok(())
}

/// Return a valid access token, doing the minimum work: cached → refresh →
/// (login if a signer is available) → else ask the user to reconnect.
pub async fn ensure_token(
    client: &RestClient,
    account: &str,
    network_label: &str,
    signer: Option<&Signer>,
    verbose: bool,
) -> Result<String> {
    let path = cache_path(network_label, account)?;
    let mut cache = load_cache(&path);

    if !cache.access_token.is_empty() && token_is_fresh(cache.access_expiry, now_unix()) {
        return Ok(cache.access_token);
    }

    if !cache.refresh_token.is_empty() && token_is_fresh(cache.refresh_expiry, now_unix()) {
        if let Ok(pair) = refresh(client, &cache.refresh_token, verbose).await {
            cache = pair;
            save_cache(&path, &cache)?;
            return Ok(cache.access_token);
        }
    }

    match signer {
        Some(s) => {
            cache = login(client, s, account, verbose).await?;
            save_cache(&path, &cache)?;
            Ok(cache.access_token)
        }
        None => Err(RisexError::Auth(
            "session expired — run `risex auth connect`".into(),
        )),
    }
}

/// Persist tokens from a login/approve response for `account` on `network`.
pub fn store_tokens(
    network_label: &str,
    account: &str,
    login_response: &serde_json::Value,
) -> Result<()> {
    let path = cache_path(network_label, account)?;
    save_cache(&path, &apply_tokens(login_response))
}

/// Clear any cached session for this account/network.
pub fn clear(network_label: &str, account: &str) -> Result<()> {
    let path = cache_path(network_label, account)?;
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

fn apply_tokens(v: &serde_json::Value) -> TokenCache {
    let now = now_unix();
    let access = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    let refresh = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    let expires_in = v.get("expires_in").and_then(|x| x.as_i64()).unwrap_or(900);
    TokenCache {
        access_token: access,
        refresh_token: refresh,
        access_expiry: now + expires_in,
        refresh_expiry: now + 7 * 24 * 3600, // refresh TTL 7d
    }
}

async fn refresh(client: &RestClient, refresh_token: &str, verbose: bool) -> Result<TokenCache> {
    let v = client
        .post_signed(
            "/v1/auth/refresh",
            json!({ "refresh_token": refresh_token }),
            verbose,
        )
        .await?;
    Ok(apply_tokens(&v))
}

async fn login(
    client: &RestClient,
    signer: &Signer,
    account: &str,
    verbose: bool,
) -> Result<TokenCache> {
    let domain = client.fetch_eip712_domain(verbose).await?;
    let nonce_resp = client
        .public_get(&format!("/v1/auth/nonce?account={account}"), &[], verbose)
        .await?;
    let nonce = nonce_resp
        .get("nonce")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RisexError::Auth("login nonce missing".into()))?;
    let deadline = (now_unix() + 300) as u32;
    let signature = signer.sign_login(&domain, account, nonce, deadline)?;
    let body = json!({ "account": account, "nonce": nonce, "deadline": deadline, "signature": signature });
    let v = client.post_signed("/v1/auth/login", body, verbose).await?;
    Ok(apply_tokens(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_until_30s_before_expiry() {
        assert!(token_is_fresh(1000, 960)); // 40s left -> fresh
        assert!(!token_is_fresh(1000, 980)); // 20s left -> stale
        assert!(!token_is_fresh(1000, 1000)); // expired
    }

    #[tokio::test]
    async fn ensure_token_without_signer_or_cache_asks_to_reconnect() {
        // Unique account so no cache file exists; never hits the network.
        let acct = "0xtest_no_session_0001";
        let client = crate::client::RestClient::new("http://127.0.0.1:1").unwrap();
        let err = ensure_token(&client, acct, "testnet", None, false)
            .await
            .unwrap_err();
        assert_eq!(err.category(), "auth");
        assert!(err.to_string().contains("auth connect"));
    }
}
