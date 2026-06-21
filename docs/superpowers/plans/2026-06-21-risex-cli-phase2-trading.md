# RISEx CLI — Phase 2+4: Auth & Trading (JWT) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user open, inspect, and close a RISEx perp position from the CLI using the JWT auth model — `auth approve` / `auth login` once, then `order buy|sell`, `positions`, `close`, `cancel`.

**Architecture:** Add an `alloy`-based EIP-712 signer (`PermitSingle` + `Login` only), a JWT session manager (nonce→login→token cache→refresh, with transparent re-login on 401), and Bearer-authenticated REST methods on the existing `RestClient`. Order commands convert human size/price to integer `size_steps`/`price_ticks` via each market's tick/step, then POST to `/v1/orders/*`. Dangerous (write) commands confirm unless `-y`, and warn loudly on mainnet.

**Tech Stack:** Rust 2021, `alloy` (signer-local, sol-types, primitives), reqwest, tokio, clap. Tests: unit (signing digests, unit conversion, session expiry), wiremock (auth + order flow).

## Global Constraints

- Binary `risex`; build stays clippy-clean; `cargo fmt` style.
- JWT path only: order requests omit the `permit` field; auth uses `Authorization: Bearer <access>`.
- Money-math and signing get full TDD; command plumbing can be lighter.
- Credentials resolve **flag > env > config**: `--private-key`/`RISEX_PRIVATE_KEY`, `--account`/`RISEX_ACCOUNT` (account derived from key if absent). Key never logged (`SecretValue`); token cache and config files are mode `0600`.
- EIP-712 domain (`name`,`version`,`chainId`,`verifyingContract`) is fetched at runtime from `GET /v1/auth/eip712-domain` per network — never hardcoded.
- Enum mappings: side 0=Buy,1=Sell; order_type 0=Market,1=Limit; tif 0=GTC,1=GTT,2=FOK,3=IOC; stp 0=ExpireMaker,1=ExpireTaker,2=ExpireBoth; margin_mode 0=Cross,1=Isolated.
- Market order ⇒ `price_ticks=0`, `time_in_force=3 (IOC)`.
- Write commands are confirmation-gated unless `-y`; on `mainnet` they print a red warning first.

---

### Task 1: Add alloy + EIP-712 signing module

**Files:**
- Modify: `Cargo.toml` (add `alloy`, `hex`)
- Create: `src/signing.rs`
- Modify: `src/lib.rs` (add `pub mod signing;`)
- Test: inline `#[cfg(test)]` in `src/signing.rs`

**Interfaces:**
- Produces:
  - `pub struct Eip712Domain { pub name: String, pub version: String, pub chain_id: u64, pub verifying_contract: String }`
  - `pub struct Signer { /* wraps alloy PrivateKeySigner */ }`
  - `pub fn Signer::from_key(private_key: &str) -> Result<Signer>` (accepts `0x`-prefixed or bare hex)
  - `pub fn Signer::address(&self) -> String` (checksummed `0x…`)
  - `pub fn Signer::sign_permit_single(&self, d: &Eip712Domain, account: &str, operator: &str, budget: u128, allowance_expiry: u32, nonce_anchor: u64, nonce_bitmap: u8) -> Result<String>` (returns `0x…` 65-byte sig, v∈{27,28})
  - `pub fn Signer::sign_login(&self, d: &Eip712Domain, account: &str, nonce_hex: &str, deadline: u32) -> Result<String>`

- [ ] **Step 1: Add dependencies**

In `Cargo.toml` `[dependencies]`:
```toml
alloy = { version = "1", default-features = false, features = ["signer-local", "sol-types", "dyn-abi"] }
hex = "0.4"
```

- [ ] **Step 2: Write failing tests**

```rust
// bottom of src/signing.rs
#[cfg(test)]
mod tests {
    use super::*;

    // Well-known Hardhat account #0.
    const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const ADDR: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    fn domain() -> Eip712Domain {
        Eip712Domain {
            name: "RISEx".into(),
            version: "1".into(),
            chain_id: 11155931,
            verifying_contract: "0x6DA86F486b5E6536358F5b122dBe184522CA0eE3".into(),
        }
    }

    #[test]
    fn derives_checksummed_address() {
        let s = Signer::from_key(KEY).unwrap();
        assert_eq!(s.address(), ADDR);
    }

    #[test]
    fn from_key_accepts_bare_hex() {
        let bare = KEY.trim_start_matches("0x");
        assert_eq!(Signer::from_key(bare).unwrap().address(), ADDR);
    }

    #[test]
    fn login_sig_is_65_bytes_with_valid_v() {
        let s = Signer::from_key(KEY).unwrap();
        let sig = s
            .sign_login(&domain(), ADDR, "0x23c6560f9a08ad3e2fab7b75ca6c36417c3242799b241f7706bf0e7f15c075a7", 1778573048)
            .unwrap();
        assert!(sig.starts_with("0x"));
        let bytes = hex::decode(sig.trim_start_matches("0x")).unwrap();
        assert_eq!(bytes.len(), 65);
        assert!(bytes[64] == 27 || bytes[64] == 28, "v must be 27/28, got {}", bytes[64]);
    }

    #[test]
    fn permit_sig_is_deterministic() {
        let s = Signer::from_key(KEY).unwrap();
        let a = s.sign_permit_single(&domain(), ADDR, ADDR, 1_000_000_000_000_000_000_000, 1781164860, 0, 1).unwrap();
        let b = s.sign_permit_single(&domain(), ADDR, ADDR, 1_000_000_000_000_000_000_000, 1781164860, 0, 1).unwrap();
        assert_eq!(a, b);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test signing::`
Expected: FAIL — module/types not defined.

- [ ] **Step 4: Implement `src/signing.rs`**

```rust
//! EIP-712 signing for the JWT auth flow. Only two typed structs are needed:
//! `PermitSingle` (one-time ApproveSingle) and `Login` (per session).
use alloy::primitives::{Address, U256};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use alloy::sol_types::{eip712_domain, sol, SolStruct};

use crate::errors::{Result, RisexError};

sol! {
    #[allow(missing_docs)]
    struct PermitSingle {
        address account;
        address operator;
        uint96 budget;
        uint32 allowanceExpiry;
        uint48 nonceAnchor;
        uint8 nonceBitmap;
    }

    #[allow(missing_docs)]
    struct Login {
        address account;
        uint256 nonce;
        uint32 deadline;
    }
}

/// Runtime EIP-712 domain (fetched from the API per network).
pub struct Eip712Domain {
    pub name: String,
    pub version: String,
    pub chain_id: u64,
    pub verifying_contract: String,
}

fn parse_addr(s: &str) -> Result<Address> {
    s.parse::<Address>()
        .map_err(|e| RisexError::Signing(format!("invalid address '{s}': {e}")))
}

fn build_domain(d: &Eip712Domain) -> Result<alloy::sol_types::Eip712Domain> {
    Ok(eip712_domain! {
        name: d.name.clone(),
        version: d.version.clone(),
        chain_id: d.chain_id,
        verifying_contract: parse_addr(&d.verifying_contract)?,
    })
}

pub struct Signer {
    inner: PrivateKeySigner,
}

impl Signer {
    pub fn from_key(private_key: &str) -> Result<Self> {
        let key = private_key.trim();
        let key = key.strip_prefix("0x").unwrap_or(key);
        let inner = key
            .parse::<PrivateKeySigner>()
            .map_err(|e| RisexError::Signing(format!("invalid private key: {e}")))?;
        Ok(Self { inner })
    }

    pub fn address(&self) -> String {
        self.inner.address().to_checksum(None)
    }

    fn finalize(&self, hash: alloy::primitives::B256) -> Result<String> {
        let sig = self
            .inner
            .sign_hash_sync(&hash)
            .map_err(|e| RisexError::Signing(format!("signing failed: {e}")))?;
        let mut bytes = sig.as_bytes().to_vec(); // [r(32) | s(32) | v(1)]
        if bytes.len() == 65 && bytes[64] < 27 {
            bytes[64] += 27; // normalize y-parity 0/1 -> 27/28
        }
        Ok(format!("0x{}", hex::encode(bytes)))
    }

    pub fn sign_permit_single(
        &self,
        d: &Eip712Domain,
        account: &str,
        operator: &str,
        budget: u128,
        allowance_expiry: u32,
        nonce_anchor: u64,
        nonce_bitmap: u8,
    ) -> Result<String> {
        let domain = build_domain(d)?;
        let msg = PermitSingle {
            account: parse_addr(account)?,
            operator: parse_addr(operator)?,
            budget: budget.try_into().map_err(|_| RisexError::Signing("budget overflow".into()))?,
            allowanceExpiry: allowance_expiry,
            nonceAnchor: U256::from(nonce_anchor).to(),
            nonceBitmap: nonce_bitmap,
        };
        self.finalize(msg.eip712_signing_hash(&domain))
    }

    pub fn sign_login(
        &self,
        d: &Eip712Domain,
        account: &str,
        nonce_hex: &str,
        deadline: u32,
    ) -> Result<String> {
        let domain = build_domain(d)?;
        let nonce = U256::from_str_radix(nonce_hex.trim_start_matches("0x"), 16)
            .map_err(|e| RisexError::Signing(format!("invalid nonce '{nonce_hex}': {e}")))?;
        let msg = Login {
            account: parse_addr(account)?,
            nonce,
            deadline,
        };
        self.finalize(msg.eip712_signing_hash(&domain))
    }
}
```

Add to `src/lib.rs` module list: `pub mod signing;`

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test signing::`
Expected: PASS (4 tests). If `nonceAnchor`/`budget` type coercions don't compile, adjust to the exact alloy `Uint<N>` constructors the compiler names (e.g. `alloy::primitives::aliases::U48`); the four tests pin behavior.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/signing.rs src/lib.rs
git commit -m "feat: add alloy EIP-712 signer for PermitSingle and Login"
```

---

### Task 2: Credential resolution

**Files:**
- Modify: `src/config.rs` (add resolver + tests)

**Interfaces:**
- Consumes: `signing::Signer` (to derive the account address from the key).
- Produces:
  - `pub struct Credentials { pub private_key: SecretValue, pub account: String }`
  - `pub fn resolve_credentials(flag_key: Option<&str>, flag_account: Option<&str>) -> Result<Credentials>` — precedence flag>env>config; derives `account` from the key when not given; errors clearly when no key found.

- [ ] **Step 1: Write the failing test**

```rust
// in src/config.rs tests
#[test]
fn resolve_prefers_flag_key_and_derives_account() {
    let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let c = resolve_credentials(Some(key), None).unwrap();
    assert_eq!(c.account, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    assert_eq!(c.private_key.expose(), key);
}

#[test]
fn resolve_errors_when_no_key() {
    // Ensure no env var leaks in.
    std::env::remove_var("RISEX_PRIVATE_KEY");
    let err = resolve_credentials(None, None);
    // With no config file in a clean test env this should be an auth error.
    assert!(err.is_err());
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test config::tests::resolve`
Expected: FAIL — `resolve_credentials` not defined.

- [ ] **Step 3: Implement**

```rust
// add near the bottom of src/config.rs (before #[cfg(test)])
use crate::signing::Signer;

pub struct Credentials {
    pub private_key: SecretValue,
    pub account: String,
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

/// Resolve trading credentials: flag > env > config. The account address is
/// derived from the private key when not explicitly provided.
pub fn resolve_credentials(
    flag_key: Option<&str>,
    flag_account: Option<&str>,
) -> Result<Credentials> {
    let key = flag_key
        .map(|s| s.to_string())
        .or_else(|| env_nonempty("RISEX_PRIVATE_KEY"))
        .or_else(|| load().ok().and_then(|c| c.auth.private_key));
    let key = key.ok_or_else(|| {
        RisexError::Auth(
            "No private key found. Use `risex auth import --private-key 0x…`, set \
             RISEX_PRIVATE_KEY, or pass --private-key."
                .into(),
        )
    })?;

    // Validate + derive address.
    let signer = Signer::from_key(&key)?;
    let derived = signer.address();

    let account = flag_account
        .map(|s| s.to_string())
        .or_else(|| env_nonempty("RISEX_ACCOUNT"))
        .or_else(|| load().ok().and_then(|c| c.auth.account))
        .unwrap_or(derived);

    Ok(Credentials {
        private_key: SecretValue::new(key),
        account,
    })
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test config::tests::resolve`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat: resolve trading credentials (flag>env>config) with derived account"
```

---

### Task 3: Auth REST methods + runtime domain/system fetch

**Files:**
- Modify: `src/client.rs` (add bearer + signed-post methods, plus runtime domain/system fetch)

**Interfaces:**
- Produces on `RestClient`:
  - `pub async fn post_signed(&self, path: &str, body: serde_json::Value, verbose: bool) -> Result<Value>` — POST JSON, no bearer (for approve/login/refresh).
  - `pub async fn get_bearer(&self, path: &str, query: &[(&str,&str)], token: &str, verbose: bool) -> Result<Value>`
  - `pub async fn post_bearer(&self, path: &str, body: serde_json::Value, token: &str, verbose: bool) -> Result<Value>`
  - `pub async fn fetch_eip712_domain(&self, verbose: bool) -> Result<crate::signing::Eip712Domain>` (GET `/v1/auth/eip712-domain`)
  - `pub async fn fetch_operator_hub(&self, verbose: bool) -> Result<String>` (GET `/v1/system/config` → `addresses.operator_hub`)
  - `pub async fn fetch_nonce_state(&self, account: &str, verbose: bool) -> Result<(u64, u8)>` (GET `/v1/nonce-state/{account}` → `(nonce_anchor, current_bitmap_index)`)

- [ ] **Step 1: Write the failing test (wiremock)**

```rust
// tests/auth_client.rs
use risex_cli::client::RestClient;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn fetch_domain_parses_fields() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/auth/eip712-domain"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"name":"RISEx","version":"1","chain_id":"11155931","verifying_contract":"0x6DA86F486b5E6536358F5b122dBe184522CA0eE3"}
        })))
        .mount(&s).await;
    let c = RestClient::new(&s.uri()).unwrap();
    let d = c.fetch_eip712_domain(false).await.unwrap();
    assert_eq!(d.name, "RISEx");
    assert_eq!(d.chain_id, 11155931);
}

#[tokio::test]
async fn post_bearer_sends_authorization_header() {
    use wiremock::matchers::header;
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/orders/place")).and(header("authorization","Bearer tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"order_id":"1-2-0"}})))
        .mount(&s).await;
    let c = RestClient::new(&s.uri()).unwrap();
    let out = c.post_bearer("/v1/orders/place", json!({"market_id":1}), "tok", false).await.unwrap();
    assert_eq!(out["order_id"], "1-2-0");
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --test auth_client`
Expected: FAIL — methods not defined.

- [ ] **Step 3: Implement the methods**

```rust
// in src/client.rs, inside impl RestClient
pub async fn post_signed(&self, path: &str, body: serde_json::Value, verbose: bool) -> Result<Value> {
    self.send_json(reqwest::Method::POST, path, &body, None, verbose).await
}

pub async fn post_bearer(&self, path: &str, body: serde_json::Value, token: &str, verbose: bool) -> Result<Value> {
    self.send_json(reqwest::Method::POST, path, &body, Some(token), verbose).await
}

pub async fn get_bearer(&self, path: &str, query: &[(&str, &str)], token: &str, verbose: bool) -> Result<Value> {
    let url = format!("{}{}", self.base_url, path);
    if verbose { output::verbose(&format!("GET {url} {query:?} (bearer)")); }
    let mut req = self.http.get(&url).query(query).bearer_auth(token);
    for (k, v) in telemetry::client_headers() { req = req.header(k, v); }
    let resp = req.send().await.map_err(RisexError::from)?;
    let status = resp.status();
    let body = resp.text().await.map_err(RisexError::from)?;
    parse_envelope(status, &body)
}

// Shared JSON-body sender (POST). Mutations are NOT retried.
async fn send_json(&self, m: reqwest::Method, path: &str, body: &serde_json::Value, token: Option<&str>, verbose: bool) -> Result<Value> {
    let url = format!("{}{}", self.base_url, path);
    if verbose { output::verbose(&format!("{m} {url} {body}")); }
    let mut req = self.http.request(m, &url).json(body);
    if let Some(t) = token { req = req.bearer_auth(t); }
    for (k, v) in telemetry::client_headers() { req = req.header(k, v); }
    let resp = req.send().await.map_err(RisexError::from)?;
    let status = resp.status();
    let text = resp.text().await.map_err(RisexError::from)?;
    parse_envelope(status, &text)
}

pub async fn fetch_eip712_domain(&self, verbose: bool) -> Result<crate::signing::Eip712Domain> {
    let d = self.public_get("/v1/auth/eip712-domain", &[], verbose).await?;
    let get = |k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let chain_id = get("chain_id").parse::<u64>()
        .map_err(|_| RisexError::Parse("bad chain_id in eip712-domain".into()))?;
    Ok(crate::signing::Eip712Domain {
        name: get("name"), version: get("version"), chain_id,
        verifying_contract: get("verifying_contract"),
    })
}

pub async fn fetch_operator_hub(&self, verbose: bool) -> Result<String> {
    let d = self.public_get("/v1/system/config", &[], verbose).await?;
    d.get("addresses").and_then(|a| a.get("operator_hub")).and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| RisexError::Parse("operator_hub missing in system config".into()))
}

pub async fn fetch_nonce_state(&self, account: &str, verbose: bool) -> Result<(u64, u8)> {
    let d = self.public_get(&format!("/v1/nonce-state/{account}"), &[], verbose).await?;
    let anchor = d.get("nonce_anchor").and_then(|v| v.as_str()).unwrap_or("0").parse::<u64>().unwrap_or(0);
    let idx = d.get("current_bitmap_index").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    Ok((anchor, idx))
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --test auth_client`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/client.rs tests/auth_client.rs
git commit -m "feat: add bearer/signed REST methods + runtime domain/operator/nonce fetch"
```

---

### Task 4: JWT session manager

**Files:**
- Create: `src/session.rs`
- Modify: `src/lib.rs` (`pub mod session;`)
- Test: inline `#[cfg(test)]` (token-expiry logic) + covered end-to-end in Task 8.

**Interfaces:**
- Consumes: `RestClient`, `signing::Signer`, `config::{Credentials, data_dir}`.
- Produces:
  - `pub struct Session { /* token cache path, in-memory tokens */ }`
  - `pub async fn Session::ensure_token(client: &RestClient, signer: &Signer, account: &str, network_label: &str, verbose: bool) -> Result<String>` — returns a valid access token: use cached if >30s from expiry; else refresh; else full Login. Persists rotated tokens to `~/.local/share/risex/session-<network>-<account>.json` (0600).
  - `pub fn token_is_fresh(expiry_unix: i64, now_unix: i64) -> bool` (pure, tested)

- [ ] **Step 1: Write the failing test**

```rust
// bottom of src/session.rs
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_until_30s_before_expiry() {
        assert!(token_is_fresh(1000, 960));   // 40s left -> fresh
        assert!(!token_is_fresh(1000, 980));  // 20s left -> stale
        assert!(!token_is_fresh(1000, 1000)); // expired
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test session::`
Expected: FAIL — module not defined.

- [ ] **Step 3: Implement**

```rust
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
    access_expiry: i64,  // unix seconds
    refresh_expiry: i64,
}

pub fn token_is_fresh(expiry_unix: i64, now_unix: i64) -> bool {
    expiry_unix - now_unix > 30
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

fn cache_path(network_label: &str, account: &str) -> Result<PathBuf> {
    Ok(config::data_dir()?.join(format!("session-{network_label}-{}.json", account.to_lowercase())))
}

fn load_cache(p: &PathBuf) -> TokenCache {
    fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_cache(p: &PathBuf, c: &TokenCache) -> Result<()> {
    if let Some(dir) = p.parent() { fs::create_dir_all(dir)?; }
    let data = serde_json::to_string(c)?;
    write_0600(p, data.as_bytes())
}

#[cfg(unix)]
fn write_0600(p: &PathBuf, data: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(p)?;
    f.write_all(data)?;
    Ok(())
}
#[cfg(not(unix))]
fn write_0600(p: &PathBuf, data: &[u8]) -> Result<()> { fs::write(p, data)?; Ok(()) }

/// Return a valid access token, doing the minimum work: cached → refresh → login.
pub async fn ensure_token(
    client: &RestClient,
    signer: &Signer,
    account: &str,
    network_label: &str,
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

    cache = login(client, signer, account, verbose).await?;
    save_cache(&path, &cache)?;
    Ok(cache.access_token)
}

fn apply_tokens(v: &serde_json::Value) -> TokenCache {
    let now = now_unix();
    let access = v.get("access_token").and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let refresh = v.get("refresh_token").and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let expires_in = v.get("expires_in").and_then(|x| x.as_i64()).unwrap_or(900);
    TokenCache {
        access_token: access,
        refresh_token: refresh,
        access_expiry: now + expires_in,
        refresh_expiry: now + 7 * 24 * 3600, // refresh TTL 7d
    }
}

async fn refresh(client: &RestClient, refresh_token: &str, verbose: bool) -> Result<TokenCache> {
    let v = client.post_signed("/v1/auth/refresh", json!({ "refresh_token": refresh_token }), verbose).await?;
    Ok(apply_tokens(&v))
}

async fn login(client: &RestClient, signer: &Signer, account: &str, verbose: bool) -> Result<TokenCache> {
    let domain = client.fetch_eip712_domain(verbose).await?;
    let nonce_resp = client.public_get(&format!("/v1/auth/nonce?account={account}"), &[], verbose).await?;
    let nonce = nonce_resp.get("nonce").and_then(|v| v.as_str())
        .ok_or_else(|| RisexError::Auth("login nonce missing".into()))?;
    let deadline = (now_unix() + 300) as u32;
    let signature = signer.sign_login(&domain, account, nonce, deadline)?;
    let body = json!({ "account": account, "nonce": nonce, "deadline": deadline, "signature": signature });
    let v = client.post_signed("/v1/auth/login", body, verbose).await?;
    Ok(apply_tokens(&v))
}
```

Add `pub mod session;` to `src/lib.rs`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test session::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/session.rs src/lib.rs
git commit -m "feat: add JWT session manager (login/refresh/0600 cache)"
```

---

### Task 5: Unit conversion (size↔steps, price↔ticks)

**Files:**
- Create: `src/commands/trade.rs` (start with conversion helpers + tests)
- Modify: `src/commands/mod.rs` (`pub mod trade;`)

**Interfaces:**
- Produces:
  - `pub fn size_to_steps(size: f64, step_size: f64) -> Result<u32>` (rounds to nearest, errors if step ≤ 0 or result 0/overflow)
  - `pub fn price_to_ticks(price: f64, step_price: f64) -> Result<u32>` (rounds to nearest; errors on overflow > 16_777_215)

- [ ] **Step 1: Write the failing test**

```rust
// in src/commands/trade.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_steps_round_to_nearest() {
        assert_eq!(size_to_steps(0.01, 0.000001).unwrap(), 10_000);
        assert_eq!(size_to_steps(0.1, 0.001).unwrap(), 100);
    }

    #[test]
    fn price_ticks_round_to_nearest() {
        assert_eq!(price_to_ticks(64000.0, 0.1).unwrap(), 640_000);
        assert_eq!(price_to_ticks(1700.55, 0.01).unwrap(), 170_055);
    }

    #[test]
    fn price_ticks_overflow_errors() {
        assert!(price_to_ticks(2_000_000.0, 0.1).is_err()); // > uint24
    }

    #[test]
    fn zero_size_errors() {
        assert!(size_to_steps(0.0, 0.001).is_err());
    }
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test trade::tests`
Expected: FAIL — functions not defined.

- [ ] **Step 3: Implement**

```rust
//! Trading commands (JWT path): place/cancel orders, positions, close, etc.
use crate::errors::{Result, RisexError};

const MAX_TICKS: u64 = 16_777_215; // uint24

pub fn size_to_steps(size: f64, step_size: f64) -> Result<u32> {
    if step_size <= 0.0 {
        return Err(RisexError::Validation("market step_size is not positive".into()));
    }
    let steps = (size / step_size).round();
    if steps < 1.0 {
        return Err(RisexError::Validation(format!(
            "size {size} is below one step ({step_size})"
        )));
    }
    if steps > u32::MAX as f64 {
        return Err(RisexError::Validation("size too large".into()));
    }
    Ok(steps as u32)
}

pub fn price_to_ticks(price: f64, step_price: f64) -> Result<u32> {
    if step_price <= 0.0 {
        return Err(RisexError::Validation("market step_price is not positive".into()));
    }
    let ticks = (price / step_price).round();
    if ticks < 0.0 || ticks as u64 > MAX_TICKS {
        return Err(RisexError::Validation(format!(
            "price {price} out of range for tick {step_price}"
        )));
    }
    Ok(ticks as u32)
}
```

Add `pub mod trade;` to `src/commands/mod.rs`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test trade::tests`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/commands/trade.rs src/commands/mod.rs
git commit -m "feat: add size->steps / price->ticks conversion with bounds checks"
```

---

### Task 6: `auth` command group

**Files:**
- Create: `src/commands/auth.rs`
- Modify: `src/commands/mod.rs`, `src/lib.rs` (Command variant, dispatch, AppContext creds)

**Interfaces:**
- Produces `pub enum AuthCommand { Import {..}, Status, Login, Approve { budget: f64, expiry: Option<String> }, Allowance, Refresh, Reset }` and `pub async fn auth::execute(cmd: &AuthCommand, ctx: &AppContext) -> Result<CommandOutput>`.
- AppContext gains `pub private_key: Option<String>` and `pub account: Option<String>` (populated in `main.rs` from flag/env, like `api_url`).

This task wires real account access. Detailed behavior:

- `auth import --private-key 0x… [--account 0x…]` → validate key via `Signer::from_key`, write `config.auth.private_key`(+account) via `config::save` (0600), print derived/checksummed account (masked).
- `auth status` → resolve creds; print account, network, then `GET /v1/auth/allowance-status?account=…` (status: active|expired|not_approved) and whether a fresh session token is cached.
- `auth login` → force `session::ensure_token` (skips straight to login if no fresh token); print "session active, expires in Nm".
- `auth approve --budget <usd> [--expiry 30d]` → fetch operator_hub + domain + nonce-state; `budget_wad = usd * 1e18` (as u128); `allowance_expiry = now + parse_duration(expiry|default 30d)`; sign `PermitSingle`; `POST /v1/auth/approve-single` with `{account, operator, budget, allowance_expiry, nonce_anchor, nonce_bitmap_index, signature}`; print tx_hash. **Dangerous + mainnet warning.**
- `auth allowance` → `GET /v1/auth/allowance-status?account=…`, print status + expiry.
- `auth refresh` → force a refresh/login; print new expiry.
- `auth reset` → delete token cache file(s) and clear `config.auth`.

- [ ] **Step 1: Write the failing test (wiremock, approve flow)**

```rust
// tests/auth_cmd.rs — drives `auth approve` against a mock and asserts the signed POST shape.
use assert_cmd::Command;
use serde_json::json;
use wiremock::matchers::{method, path, body_partial_json};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ACCT: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

#[tokio::test]
async fn auth_approve_posts_signed_permit() {
    let s = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/auth/eip712-domain"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"name":"RISEx","version":"1","chain_id":"11155931","verifying_contract":"0x6DA86F486b5E6536358F5b122dBe184522CA0eE3"}}))).mount(&s).await;
    Mock::given(method("GET")).and(path("/v1/system/config"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"addresses":{"operator_hub":"0x0AbF5B4CDd7B1ae4f444e4Ab5E98b341567e3402"}}}))).mount(&s).await;
    Mock::given(method("GET")).and(path(format!("/v1/nonce-state/{ACCT}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"nonce_anchor":"0","current_bitmap_index":1}}))).mount(&s).await;
    Mock::given(method("POST")).and(path("/v1/auth/approve-single"))
        .and(body_partial_json(json!({"account":ACCT})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"transaction_hash":"0xabc","success":true}}))).mount(&s).await;

    let uri = s.uri();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("risex").unwrap()
            .args(["--api-url",&uri,"--private-key",KEY,"-y","auth","approve","--budget","1000"])
            .assert().success().stdout(predicates::str::contains("0xabc"));
    }).await.unwrap();
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --test auth_cmd`
Expected: FAIL — `auth` command missing.

- [ ] **Step 3: Implement `src/commands/auth.rs`, wire Command + AppContext**

Implement `AuthCommand` + `execute` per the behavior list above. In `main.rs`, populate `ctx.private_key`/`ctx.account` from `--private-key`/`RISEX_PRIVATE_KEY` and `--account`/`RISEX_ACCOUNT`. In `lib.rs`, add the `Auth { #[command(subcommand)] cmd: AuthCommand }` variant and dispatch to `auth::execute(&cmd, ctx)`. Add a helper `ctx.signer()? -> Signer` and `ctx.credentials()? -> Credentials`. Key helper for approve:

```rust
// duration parse: "30d","12h","3600s","1781164860"(absolute unix) -> allowance_expiry unix
fn resolve_expiry(arg: Option<&str>) -> Result<u32> {
    let now = chrono::Utc::now().timestamp();
    let secs = match arg {
        None => 30 * 24 * 3600,
        Some(s) => {
            let s = s.trim();
            if let Some(n) = s.strip_suffix('d') { n.parse::<i64>().map_err(bad)? * 86400 }
            else if let Some(n) = s.strip_suffix('h') { n.parse::<i64>().map_err(bad)? * 3600 }
            else if let Some(n) = s.strip_suffix('s') { n.parse::<i64>().map_err(bad)? }
            else { let v = s.parse::<i64>().map_err(bad)?; return Ok(v as u32); } // absolute
        }
    };
    Ok((now + secs) as u32)
}
fn bad<E: std::fmt::Display>(e: E) -> RisexError { RisexError::Validation(format!("bad expiry: {e}")) }
```

`budget_wad`: `let wad = (usd * 1e18_f64) as u128;` (acceptable precision for USD budgets).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --test auth_cmd`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/commands/auth.rs src/commands/mod.rs src/lib.rs src/main.rs tests/auth_cmd.rs
git commit -m "feat: add auth command group (import/status/login/approve/allowance/refresh/reset)"
```

---

### Task 7: `order` commands + positions/balance/close

**Files:**
- Modify: `src/commands/trade.rs` (add command enum + handlers)
- Modify: `src/lib.rs` (Command variants + dispatch), `src/commands/mod.rs`

**Interfaces:**
- Produces `pub enum OrderCommand { Buy{..}, Sell{..}, Cancel{..}, CancelAll{..} }`, plus top-level `Positions`, `Balance`, `Close`, `Leverage`, `Margin` handlers, and `pub async fn trade::execute_order(...)`.
- Consumes: `session::ensure_token`, `RestClient::{post_bearer,get_bearer}`, `resolve_market` (market object for step/tick), `size_to_steps`, `price_to_ticks`.

Order placement handler (the core):

```rust
// pseudo-flow inside the buy/sell handler
let m = resolve_market(client, &market, verbose).await?;          // reuse from market.rs (make it pub(crate))
let market_id: u32 = s(&m,"market_id").parse()?;
let step_size: f64 = m["config"]["step_size"].as_str().unwrap_or("0").parse()?;
let step_price: f64 = m["config"]["step_price"].as_str().unwrap_or("0").parse()?;
let size_steps = size_to_steps(size, step_size)?;
let (order_type, price_ticks, tif) = if market_order {
    (0u32, 0u32, 3u32)                                            // Market => IOC, price 0
} else {
    (1u32, price_to_ticks(price.ok_or(/* price required */)?, step_price)?, tif_flag)
};
let token = session::ensure_token(client, &signer, &account, network.label(), verbose).await?;
let body = json!({
    "market_id": market_id, "size_steps": size_steps, "price_ticks": price_ticks,
    "side": side, "post_only": post_only, "reduce_only": reduce_only,
    "stp_mode": stp, "order_type": order_type, "time_in_force": tif,
    "builder_id": 0, "client_order_id": 0, "ttl_units": 0
}); // NOTE: no "permit" -> JWT path
let resp = client.post_bearer("/v1/orders/place", body, &token, verbose).await?;
// render order_id, tx_hash, filled_percent (IOC)
```

- `cancel <market> <order_id>` → `POST /v1/orders/cancel` `{market_id, order_id}` (bearer).
- `cancel-all [--market]` → `POST /v1/orders/cancel-all` `{market_id}` (bearer; market required, >0).
- `positions [--market]` → `GET /v1/positions?account=…` (bearer); table: Market, Side, Size, Entry, Mark, uPnL, Liq, Lev.
- `balance` → `GET /v1/account/balance?account=…` (bearer); key-value.
- `close <market>` → look up the open position size/side; submit a market `reduce_only` order on the opposite side for the full size.
- `leverage <market> <x>` → `POST /v1/account/leverage` `{market_id, leverage}` (bearer; if it 4xx-requires a permit, surface a clear message that leverage needs the permit path — not yet supported).
- `margin <market> cross|isolated` → `POST /v1/account/margin-mode` `{market_id, margin_mode}` (bearer, same caveat).

All write commands: `confirm_write(ctx, &summary)?` — prints the action, and on mainnet a red warning, then prompts unless `ctx.force`.

- [ ] **Step 1: Write the failing test (wiremock, place market buy)**

```rust
// tests/order_cmd.rs
use assert_cmd::Command;
use serde_json::json;
use wiremock::matchers::{method, path, body_partial_json, header};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

async fn mount_auth(s: &MockServer) {
    Mock::given(method("GET")).and(path("/v1/auth/eip712-domain")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"name":"RISEx","version":"1","chain_id":"11155931","verifying_contract":"0x6DA86F486b5E6536358F5b122dBe184522CA0eE3"}}))).mount(s).await;
    Mock::given(method("GET")).and(wiremock::matchers::path_regex(r"/v1/auth/nonce.*")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"nonce":"0x23c6560f9a08ad3e2fab7b75ca6c36417c3242799b241f7706bf0e7f15c075a7"}}))).mount(s).await;
    Mock::given(method("POST")).and(path("/v1/auth/login")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"access_token":"tok","refresh_token":"r","expires_in":900}}))).mount(s).await;
}

#[tokio::test]
async fn market_buy_places_jwt_order() {
    let s = MockServer::start().await;
    mount_auth(&s).await;
    Mock::given(method("GET")).and(path("/v1/markets")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"markets":[{"market_id":"1","display_name":"BTC/USDC","visible":true,"config":{"step_size":"0.000001","step_price":"0.1"}}]}}))).mount(&s).await;
    Mock::given(method("POST")).and(path("/v1/orders/place"))
        .and(header("authorization","Bearer tok"))
        .and(body_partial_json(json!({"market_id":1,"side":0,"order_type":0,"time_in_force":3,"size_steps":10000})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"order_id":"12345-100-0","tx_hash":"0xdead"}}))).mount(&s).await;

    let uri = s.uri();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("risex").unwrap()
            .args(["--api-url",&uri,"-n","testnet","--private-key",KEY,"-y","order","buy","btc","0.01","--type","market"])
            .assert().success().stdout(predicates::str::contains("12345-100-0"));
    }).await.unwrap();
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --test order_cmd`
Expected: FAIL — `order` command missing.

- [ ] **Step 3: Implement order commands + wiring**

Add `OrderCommand` and the `Order { cmd }`, `Positions`, `Balance`, `Close`, `Leverage`, `Margin` variants to `Command`; dispatch to handlers in `trade.rs`. Make `market::resolve_market` and `market::s` `pub(crate)` for reuse. Implement `confirm_write`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test --test order_cmd`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/commands/trade.rs src/commands/market.rs src/lib.rs src/commands/mod.rs tests/order_cmd.rs
git commit -m "feat: add order place/cancel, positions, balance, close, leverage, margin (JWT)"
```

---

### Task 8: Full-flow integration test + live testnet runbook

**Files:**
- Create: `tests/trading_flow.rs`
- Modify: `README.md` (trading usage)

**Interfaces:** none new — exercises the whole path against wiremock.

- [ ] **Step 1: Write the test** — mount domain/nonce/login + markets + `/v1/orders/place` + `/v1/positions` + `/v1/orders/cancel`; run `order buy`, then `positions`, then `order cancel`, asserting each succeeds and the bearer token flows through. (Use the patterns from Task 7.)

- [ ] **Step 2: Run** `cargo test --test trading_flow` → PASS.

- [ ] **Step 3: Add the testnet runbook to README**

```md
## Trading (testnet)

1. risex auth import --private-key 0xYOURKEY
2. risex -n testnet auth approve --budget 1000      # one-time, signs PermitSingle
3. risex -n testnet auth status                     # allowance: active
4. risex -n testnet order buy btc 0.001 --type market
5. risex -n testnet positions
6. risex -n testnet close btc
```

- [ ] **Step 4: Commit**

```bash
git add tests/trading_flow.rs README.md
git commit -m "test: full JWT trading flow + testnet runbook"
```

---

## Self-Review

**Spec coverage:** Auth/session (spec §2,§12) → Tasks 1,3,4,6. Signing (§6) → Task 1. Credentials (§4) → Task 2. Order units/encoding (§7) → Tasks 5,7. Trade commands (§8) → Task 7. Confirmation gating + mainnet safety (§8,§10) → Tasks 6,7. Runtime domain/operator fetch (§3) → Task 3. Token cache (§4) → Task 4. Out of scope here (own later plans): `auth revoke` (on-chain), WebSocket, paper, MCP, deposit, the per-op permit path for leverage/margin if Bearer is rejected.

**Placeholder scan:** No "TBD"/"add error handling" left; the one runtime risk (exact alloy `Uint`/`as_bytes` API) is pinned by Task 1's tests and called out explicitly with the fallback.

**Type consistency:** `Signer::{from_key,address,sign_permit_single,sign_login}`, `Eip712Domain`, `Credentials`, `ensure_token`, `token_is_fresh`, `size_to_steps`/`price_to_ticks`, `post_bearer`/`get_bearer`/`post_signed`, `fetch_eip712_domain`/`fetch_operator_hub`/`fetch_nonce_state` are referenced with identical signatures across tasks.

## Risks / decisions

- **Signing correctness is critical.** Unit tests pin address derivation, sig length, and v∈{27,28}, but they can't confirm the *contract* accepts the signature (we lack a known full vector). The real proof is a live testnet `auth approve` succeeding — do that early (Task 6) before relying on it.
- **Leverage/margin may require the permit path.** Built Bearer-first; if the API rejects, the command surfaces a clear "needs permit path (not yet supported)" message rather than silently failing — a follow-up plan adds the per-op `VerifyWitness` signer + bitmap nonce manager.
- **Mainnet safety:** default network is mainnet, so every write command prints a red mainnet warning and confirms unless `-y`. The runbook uses `-n testnet` explicitly.
