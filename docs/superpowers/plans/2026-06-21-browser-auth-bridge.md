# Browser Auth Bridge — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Authenticate `risex` without pasting a private key — `risex auth connect` opens a hosted wallet-connect page, the user signs the EIP-712 payload in their wallet, and the CLI receives only the signature via a one-shot localhost callback and stores the JWT.

**Architecture:** Part A (Rust): a no-key-capable session layer, a one-shot localhost callback server (`bridge.rs`), and an `auth connect` command that opens the browser and completes the RISEx API call. Part B (web): a Vite+React+wagmi app served at `connect.risescan.io` that connects the wallet, rebuilds the same EIP-712 typed data, signs, and POSTs the signature back to the localhost callback.

**Tech Stack:** Rust (`tiny_http`, `open`, existing `alloy`/`reqwest`/`tokio`); Web (Vite, React, wagmi, viem).

## Global Constraints

- Connect URL resolves **flag > env > default**: `--connect-url` / `RISEX_CONNECT_URL` / `https://connect.risescan.io`.
- The localhost server binds `127.0.0.1` only, random port, single successful callback then stops, hard timeout 180s. A `state` (CSRF) token ties the callback to the run.
- Only a signature crosses the callback; the CLI does all RISEx API calls and stores only `account` + JWT (refresh token), mode `0600`.
- `auth connect` = login only; `auth connect --approve --budget <usd> [--expiry <dur>]` = ApproveSingle only (one signature per connect; approve is never implicit).
- The web page's EIP-712 typed data (domain, struct fields, types) MUST match `src/signing.rs` exactly: `Login{account:address, nonce:uint256, deadline:uint32}`, `PermitSingle{account:address, operator:address, budget:uint96, allowanceExpiry:uint32, nonceAnchor:uint48, nonceBitmap:uint8}`; domain `{name,version,chainId,verifyingContract}` from `GET /v1/auth/eip712-domain`.
- After connect, trading commands must work with no key for the refresh window; a dead refresh + no key returns `RisexError::Auth("session expired — run \`risex auth connect\`")`.

---

## Part A — CLI side

### Task A1: No-key session + context support

**Files:**
- Modify: `src/session.rs` (change `ensure_token` signature; add `store_tokens`)
- Modify: `src/lib.rs` (`AppContext::{account, optional_signer}`)
- Modify: `src/commands/trade.rs` (update all `ensure_token` callers)
- Modify: `src/commands/auth.rs` (update `login`/`refresh` callers)
- Test: inline in `src/session.rs`

**Interfaces:**
- Produces:
  - `pub async fn session::ensure_token(client: &RestClient, account: &str, network_label: &str, signer: Option<&Signer>, verbose: bool) -> Result<String>` — cached→refresh→(login if signer)→else reconnect error.
  - `pub fn session::store_tokens(network_label: &str, account: &str, login_response: &serde_json::Value) -> Result<()>`
  - `AppContext::account(&self) -> Result<String>` — explicit account (flag/env/config) else derived from key; errors if neither.
  - `AppContext::optional_signer(&self) -> Option<signing::Signer>` — `Some` iff a key is resolvable.

- [ ] **Step 1: Write the failing test**

```rust
// in src/session.rs tests module
#[tokio::test]
async fn ensure_token_without_signer_or_cache_asks_to_reconnect() {
    // Unique account so no cache file exists.
    let acct = "0xtest_no_session_0001";
    let client = crate::client::RestClient::new("http://127.0.0.1:1").unwrap();
    let err = ensure_token(&client, acct, "testnet", None, false).await.unwrap_err();
    assert_eq!(err.category(), "auth");
    assert!(err.to_string().contains("auth connect"));
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test session::tests::ensure_token_without_signer`
Expected: FAIL — signature mismatch (current `ensure_token` takes `&Signer`, not `Option`).

- [ ] **Step 3: Update `session.rs`**

Change the signature and the login branch; add `store_tokens`. Replace the `ensure_token` body:

```rust
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
```

- [ ] **Step 4: Update callers**

In `src/commands/trade.rs`, replace every occurrence of:
```rust
    let (signer, account) = ctx.signer_and_account()?;
    let token =
        session::ensure_token(&client, &signer, &account, ctx.network.label(), ctx.verbose).await?;
```
with:
```rust
    let account = ctx.account()?;
    let signer = ctx.optional_signer();
    let token = session::ensure_token(
        &client,
        &account,
        ctx.network.label(),
        signer.as_ref(),
        ctx.verbose,
    )
    .await?;
```

In `src/commands/auth.rs`, update `login` and `refresh` the same way (they currently call `ctx.signer_and_account()` + `ensure_token`). For `refresh`, keep `session::clear` then `ensure_token(..., ctx.optional_signer().as_ref(), ...)`.

Add to `impl AppContext` in `src/lib.rs`:
```rust
fn resolve_key(&self) -> Option<String> {
    self.private_key
        .clone()
        .or_else(|| std::env::var("RISEX_PRIVATE_KEY").ok().filter(|s| !s.is_empty()))
        .or_else(|| config::load().ok().and_then(|c| c.auth.private_key))
}

/// The account address: explicit (flag/env/config) else derived from the key.
pub fn account(&self) -> Result<String> {
    if let Some(a) = self
        .account
        .clone()
        .or_else(|| std::env::var("RISEX_ACCOUNT").ok().filter(|s| !s.is_empty()))
        .or_else(|| config::load().ok().and_then(|c| c.auth.account))
    {
        return Ok(a);
    }
    let key = self.resolve_key().ok_or_else(|| {
        errors::RisexError::Auth(
            "No account configured. Run `risex auth connect` or set --account.".into(),
        )
    })?;
    Ok(signing::Signer::from_key(&key)?.address())
}

/// A signer, only if a private key is resolvable.
pub fn optional_signer(&self) -> Option<signing::Signer> {
    self.resolve_key()
        .and_then(|k| signing::Signer::from_key(&k).ok())
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test` (session test passes; existing order/auth tests still pass with the new signature)
Expected: PASS across the suite.

- [ ] **Step 6: Commit**

```bash
git add src/session.rs src/lib.rs src/commands/trade.rs src/commands/auth.rs
git commit -m "feat: no-key session support (ensure_token takes optional signer; account from config)"
```

---

### Task A2: One-shot localhost callback server (`bridge.rs`)

**Files:**
- Modify: `Cargo.toml` (add `tiny_http`, `open`)
- Create: `src/bridge.rs`
- Modify: `src/lib.rs` (`pub mod bridge;`)
- Test: `tests/bridge.rs`

**Interfaces:**
- Produces:
  - `pub struct Bridge { /* server, port, state */ }`
  - `pub fn Bridge::start() -> Result<Bridge>` (binds 127.0.0.1:0, random `state`)
  - `pub fn Bridge::port(&self) -> u16`
  - `pub fn Bridge::state(&self) -> &str`
  - `pub fn Bridge::await_callback(self, timeout: std::time::Duration) -> Result<Callback>` (consumes self; returns on first valid POST or times out)
  - `pub struct Callback { pub action: String, pub account: String, pub fields: serde_json::Value }` (`fields` carries the action-specific signed payload)

- [ ] **Step 1: Add deps**

`Cargo.toml` `[dependencies]`:
```toml
tiny_http = "0.12"
open = "5"
```

- [ ] **Step 2: Write the failing test**

```rust
// tests/bridge.rs
use std::time::Duration;
use risex_cli::bridge::Bridge;

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
```

- [ ] **Step 3: Run to verify fail**

Run: `cargo test --test bridge`
Expected: FAIL — `bridge` module missing.

- [ ] **Step 4: Implement `src/bridge.rs`**

```rust
//! One-shot localhost callback server for the browser auth bridge.
//! Binds 127.0.0.1 on a random port, accepts a single signed POST /callback,
//! validates the CSRF `state`, and returns the parsed payload.
use std::io::Read;
use std::time::{Duration, Instant};

use serde_json::Value;
use tiny_http::{Header, Method, Response, Server};

use crate::errors::{Result, RisexError};

pub struct Bridge {
    server: Server,
    port: u16,
    state: String,
}

pub struct Callback {
    pub action: String,
    pub account: String,
    pub fields: Value,
}

fn cors_headers() -> Vec<Header> {
    vec![
        Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
        Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"POST, OPTIONS"[..]).unwrap(),
        Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type"[..]).unwrap(),
    ]
}

fn respond(req: tiny_http::Request, status: u16, body: &str) {
    let mut resp = Response::from_string(body).with_status_code(status);
    for h in cors_headers() {
        resp.add_header(h);
    }
    let _ = req.respond(resp);
}

impl Bridge {
    pub fn start() -> Result<Self> {
        let server = Server::http("127.0.0.1:0")
            .map_err(|e| RisexError::Network(format!("failed to start local server: {e}")))?;
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            _ => return Err(RisexError::Network("no TCP port for local server".into())),
        };
        Ok(Self {
            server,
            port,
            state: uuid::Uuid::new_v4().to_string(),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Block until a valid signed callback arrives or the timeout elapses.
    pub fn await_callback(self, timeout: Duration) -> Result<Callback> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RisexError::Network("auth bridge timed out".into()));
            }
            match self.server.recv_timeout(remaining) {
                Ok(Some(mut req)) => {
                    // CORS preflight.
                    if *req.method() == Method::Options {
                        respond(req, 204, "");
                        continue;
                    }
                    // Friendly root page (browser may GET it).
                    if *req.method() == Method::Get {
                        respond(req, 200, "risex auth bridge — you can close this tab.");
                        continue;
                    }
                    if *req.method() != Method::Post || !req.url().starts_with("/callback") {
                        respond(req, 404, "not found");
                        continue;
                    }
                    let mut body = String::new();
                    if req.as_reader().read_to_string(&mut body).is_err() {
                        respond(req, 400, "bad body");
                        continue;
                    }
                    let v: Value = match serde_json::from_str(&body) {
                        Ok(v) => v,
                        Err(_) => {
                            respond(req, 400, "bad json");
                            continue;
                        }
                    };
                    if v.get("state").and_then(|s| s.as_str()) != Some(&self.state) {
                        respond(req, 400, "bad state");
                        continue; // keep waiting; ignore spurious posts
                    }
                    let action = v
                        .get("action")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let account = v
                        .get("account")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    if action.is_empty() || account.is_empty() {
                        respond(req, 400, "missing action/account");
                        continue;
                    }
                    respond(req, 200, "ok");
                    return Ok(Callback {
                        action,
                        account,
                        fields: v,
                    });
                }
                Ok(None) => continue, // timeout tick
                Err(e) => return Err(RisexError::Network(format!("bridge recv error: {e}"))),
            }
        }
    }
}
```

Add `pub mod bridge;` to `src/lib.rs`.

- [ ] **Step 5: Run to verify pass**

Run: `cargo test --test bridge`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/bridge.rs src/lib.rs tests/bridge.rs
git commit -m "feat: one-shot localhost callback server for browser auth bridge"
```

---

### Task A3: `auth connect` command

**Files:**
- Modify: `src/commands/auth.rs` (add `Connect` subcommand + handler)
- Modify: `src/lib.rs` (`--connect-url` global flag; `AppContext.connect_url`)
- Modify: `src/main.rs` (resolve connect URL flag>env>default)
- Test: `tests/connect_cmd.rs`

**Interfaces:**
- Consumes: `bridge::Bridge`, `session::store_tokens`, `RestClient::post_signed`, `AppContext`.
- Produces: `AuthCommand::Connect { approve: bool, budget: Option<f64>, expiry: Option<String> }`; `AppContext.connect_url: String`.

- [ ] **Step 1: Write the failing test**

The test plays the browser: it spawns the CLI's connect handler indirectly by running the binary, then POSTs the signed login to the callback. Because the binary opens a browser we must suppress that — gate browser-open behind an env var the test sets.

```rust
// tests/connect_cmd.rs
use std::time::Duration;
use assert_cmd::cargo::cargo_bin;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn auth_connect_completes_login_from_callback() {
    let api = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/auth/login"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"access_token":"tok","refresh_token":"r","expires_in":900}})))
        .mount(&api).await;

    // Run the binary; RISEX_NO_BROWSER stops it opening a browser and makes it
    // print the callback port line we parse.
    let api_uri = api.uri();
    let child = tokio::task::spawn_blocking(move || {
        std::process::Command::new(cargo_bin("risex"))
            .args(["--api-url", &api_uri, "-n", "testnet", "auth", "connect"])
            .env("RISEX_NO_BROWSER", "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    })
    .await
    .unwrap();

    // The CLI prints "BRIDGE port=<port> state=<state>" on stderr when RISEX_NO_BROWSER is set.
    // Read it, then POST the signed callback.
    let (port, state) = read_bridge_line(&child); // helper defined in the test file
    let http = reqwest::Client::new();
    http.post(format!("http://127.0.0.1:{port}/callback"))
        .json(&json!({"state": state, "action":"login", "account":"0xabc",
                      "nonce":"0x01", "deadline": 123, "signature":"0xsig"}))
        .timeout(Duration::from_secs(5))
        .send().await.unwrap();

    let out = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap()).await.unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Connected"));
}
```

(The `read_bridge_line` helper reads child stderr line-by-line until it sees `BRIDGE port=… state=…` and parses the two values. Include it in the test file.)

- [ ] **Step 2: Run to verify fail**

Run: `cargo test --test connect_cmd`
Expected: FAIL — `connect` subcommand missing.

- [ ] **Step 3: Wire the flag + context**

In `src/lib.rs` `Cli`, add:
```rust
    /// Wallet-connect bridge URL (or RISEX_CONNECT_URL).
    #[arg(long, global = true)]
    pub connect_url: Option<String>,
```
Add `pub connect_url: String` to `AppContext`. In `src/main.rs`:
```rust
    let connect_url = cli
        .connect_url
        .clone()
        .or_else(|| std::env::var("RISEX_CONNECT_URL").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "https://connect.risescan.io".to_string());
```
and set it on the `AppContext`.

- [ ] **Step 4: Implement the `Connect` handler in `src/commands/auth.rs`**

Add the variant to `AuthCommand`:
```rust
    /// Connect a wallet in the browser (no private key needed).
    Connect {
        /// Sign a one-time ApproveSingle instead of logging in.
        #[arg(long)]
        approve: bool,
        /// Budget in USD for --approve.
        #[arg(long)]
        budget: Option<f64>,
        /// Allowance expiry for --approve (30d default, 12h, 3600s, or unix).
        #[arg(long)]
        expiry: Option<String>,
    },
```
Route it in `execute`:
```rust
        AuthCommand::Connect { approve, budget, expiry } => {
            connect(ctx, *approve, *budget, expiry.as_deref()).await
        }
```
Implement:
```rust
use crate::bridge::Bridge;
use std::time::Duration;

async fn connect(
    ctx: &AppContext,
    approve: bool,
    budget: Option<f64>,
    expiry: Option<&str>,
) -> Result<CommandOutput> {
    let action = if approve { "approve" } else { "login" };
    if approve && budget.is_none() {
        return Err(RisexError::Validation("--approve requires --budget".into()));
    }

    let bridge = Bridge::start()?;
    let port = bridge.port();
    let state = bridge.state().to_string();

    // Build the connect URL.
    let mut url = format!(
        "{}/cli?network={}&callback=http://127.0.0.1:{}&state={}&action={}",
        ctx.connect_url.trim_end_matches('/'),
        ctx.network.label(),
        port,
        state,
        action
    );
    if approve {
        url.push_str(&format!("&budget={}", budget.unwrap()));
        if let Some(e) = expiry {
            url.push_str(&format!("&expiry={e}"));
        }
    }

    // Open the browser (suppressed in tests).
    if std::env::var_os("RISEX_NO_BROWSER").is_some() {
        eprintln!("BRIDGE port={port} state={state}");
        eprintln!("Open: {url}");
    } else if open::that(&url).is_err() {
        eprintln!("Could not open a browser automatically. Open this URL:\n{url}");
    } else {
        eprintln!("Opened your browser to connect a wallet… (waiting up to 180s)");
    }

    // Await the signed callback.
    let cb = tokio::task::spawn_blocking(move || bridge.await_callback(Duration::from_secs(180)))
        .await
        .map_err(|e| RisexError::Network(format!("bridge join error: {e}")))??;

    let client = ctx.client()?;
    if cb.action == "login" {
        let body = json!({
            "account": cb.account,
            "nonce": cb.fields.get("nonce"),
            "deadline": cb.fields.get("deadline"),
            "signature": cb.fields.get("signature"),
        });
        let resp = client.post_signed("/v1/auth/login", body, ctx.verbose).await?;
        session::store_tokens(ctx.network.label(), &cb.account, &resp)?;
        persist_account(&cb.account)?;
        Ok(CommandOutput::message(&format!(
            "Connected as {} ({}).",
            mask_address(&cb.account),
            ctx.network
        )))
    } else {
        // approve: forward the signed PermitSingle fields verbatim.
        let body = json!({
            "account": cb.account,
            "operator": cb.fields.get("operator"),
            "budget": cb.fields.get("budget"),
            "allowance_expiry": cb.fields.get("allowance_expiry"),
            "nonce_anchor": cb.fields.get("nonce_anchor"),
            "nonce_bitmap_index": cb.fields.get("nonce_bitmap_index"),
            "signature": cb.fields.get("signature"),
        });
        let resp = client
            .post_signed("/v1/auth/approve-single", body, ctx.verbose)
            .await?;
        persist_account(&cb.account)?;
        Ok(CommandOutput::key_value(
            vec![(
                "transaction_hash".into(),
                crate::commands::market::s(&resp, "transaction_hash"),
            )],
            resp,
        ))
    }
}

fn persist_account(account: &str) -> Result<()> {
    let mut cfg = config::load()?;
    cfg.auth.account = Some(account.to_string());
    config::save(&cfg)
}
```

- [ ] **Step 5: Run to verify pass**

Run: `cargo test --test connect_cmd`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/commands/auth.rs src/lib.rs src/main.rs tests/connect_cmd.rs
git commit -m "feat: add `auth connect` browser bridge command"
```

---

### Task A4: README — connect usage

**Files:** Modify `README.md`.

- [ ] **Step 1: Document the no-key flow**

```md
## Connect a wallet (no private key)

```sh
risex -n testnet auth connect                 # opens browser; sign Login in your wallet
risex -n testnet auth connect --approve --budget 1000   # one-time, sign ApproveSingle
risex -n testnet order buy btc 0.001 --type market      # trades for the 7-day session
```

The CLI never sees your key — your wallet signs in the browser and only the signature returns
to a one-shot `127.0.0.1` callback. Override the page with `--connect-url` / `RISEX_CONNECT_URL`.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: document `auth connect` no-key flow"
```

---

## Part B — Web app (`web/`)

> Deployed by RISEx to `connect.risescan.io`. The Rust suite does not test this; verify with a wallet using the manual runbook (Task B3). Build: `cd web && npm install && npm run build`.

### Task B1: Scaffold the Vite + wagmi app

**Files:**
- Create: `web/package.json`, `web/vite.config.ts`, `web/index.html`, `web/tsconfig.json`
- Create: `web/src/main.tsx`, `web/src/wagmi.ts`
- Create: `web/.env.example`, `web/.gitignore`

- [ ] **Step 1: `web/package.json`**

```json
{
  "name": "risex-connect",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "@tanstack/react-query": "^5.59.0",
    "react": "^18.3.1",
    "react-dom": "^18.3.1",
    "viem": "^2.21.0",
    "wagmi": "^2.12.0"
  },
  "devDependencies": {
    "@types/react": "^18.3.0",
    "@types/react-dom": "^18.3.0",
    "@vitejs/plugin-react": "^4.3.0",
    "typescript": "^5.6.0",
    "vite": "^5.4.0"
  }
}
```

- [ ] **Step 2: `web/src/wagmi.ts` — chains + connectors**

```ts
import { http, createConfig } from 'wagmi'
import { defineChain } from 'viem'
import { injected, walletConnect, coinbaseWallet } from 'wagmi/connectors'

export const riseMainnet = defineChain({
  id: 4153,
  name: 'RISE',
  nativeCurrency: { name: 'Ether', symbol: 'ETH', decimals: 18 },
  rpcUrls: { default: { http: ['https://mainnet.riselabs.xyz'] } },
})
export const riseTestnet = defineChain({
  id: 11155931,
  name: 'RISE Testnet',
  nativeCurrency: { name: 'Ether', symbol: 'ETH', decimals: 18 },
  rpcUrls: { default: { http: ['https://testnet.riselabs.xyz'] } },
})

const projectId = import.meta.env.VITE_WALLETCONNECT_PROJECT_ID ?? ''

export const config = createConfig({
  chains: [riseMainnet, riseTestnet],
  connectors: [
    injected(),
    coinbaseWallet({ appName: 'RISEx' }),
    ...(projectId ? [walletConnect({ projectId })] : []),
  ],
  transports: {
    [riseMainnet.id]: http(),
    [riseTestnet.id]: http(),
  },
})

export const API_BASE: Record<string, string> = {
  mainnet: 'https://api.rise.trade',
  testnet: 'https://api.testnet.rise.trade',
}
```

- [ ] **Step 3: `web/src/main.tsx` shell**

```tsx
import React from 'react'
import ReactDOM from 'react-dom/client'
import { WagmiProvider } from 'wagmi'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { config } from './wagmi'
import { App } from './App'

const qc = new QueryClient()
ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <WagmiProvider config={config}>
      <QueryClientProvider client={qc}>
        <App />
      </QueryClientProvider>
    </WagmiProvider>
  </React.StrictMode>,
)
```

Plus standard `index.html` (`<div id="root">`), `vite.config.ts` (`@vitejs/plugin-react`), `tsconfig.json`, `.env.example` (`VITE_WALLETCONNECT_PROJECT_ID=`), `.gitignore` (`node_modules`, `dist`).

- [ ] **Step 4: Verify it builds**

Run: `cd web && npm install && npm run build`
Expected: a `web/dist/` is produced with no type errors.

- [ ] **Step 5: Commit**

```bash
git add web/
git commit -m "feat(web): scaffold Vite + wagmi connect app"
```

---

### Task B2: The connect flow (`web/src/App.tsx`)

**Files:** Create `web/src/App.tsx`, `web/src/sign.ts`.

- [ ] **Step 1: `web/src/sign.ts` — typed data + API calls**

```ts
import { API_BASE } from './wagmi'

export interface Params {
  network: string
  callback: string
  state: string
  action: string
  budget?: string
  expiry?: string
}

export function readParams(): Params {
  const q = new URLSearchParams(window.location.search)
  return {
    network: q.get('network') ?? 'mainnet',
    callback: q.get('callback') ?? '',
    state: q.get('state') ?? '',
    action: q.get('action') ?? 'login',
    budget: q.get('budget') ?? undefined,
    expiry: q.get('expiry') ?? undefined,
  }
}

async function apiGet(network: string, path: string): Promise<any> {
  const res = await fetch(`${API_BASE[network]}${path}`)
  const json = await res.json()
  return json.data ?? json
}

async function domain(network: string) {
  const d = await apiGet(network, '/v1/auth/eip712-domain')
  return {
    name: d.name,
    version: d.version,
    chainId: Number(d.chain_id),
    verifyingContract: d.verifying_contract as `0x${string}`,
  }
}

// Login typed data — must match src/signing.rs Login{account,nonce,deadline}.
export async function buildLogin(network: string, account: `0x${string}`) {
  const nonceResp = await apiGet(network, `/v1/auth/nonce?account=${account}`)
  const nonce = nonceResp.nonce as string
  const deadline = Math.floor(Date.now() / 1000) + 300
  const typedData = {
    domain: await domain(network),
    types: {
      Login: [
        { name: 'account', type: 'address' },
        { name: 'nonce', type: 'uint256' },
        { name: 'deadline', type: 'uint32' },
      ],
    },
    primaryType: 'Login' as const,
    message: { account, nonce: BigInt(nonce), deadline },
  }
  return { typedData, nonce, deadline }
}

function expirySeconds(expiry?: string): number {
  const now = Math.floor(Date.now() / 1000)
  if (!expiry) return now + 30 * 24 * 3600
  if (expiry.endsWith('d')) return now + parseInt(expiry) * 86400
  if (expiry.endsWith('h')) return now + parseInt(expiry) * 3600
  if (expiry.endsWith('s')) return now + parseInt(expiry)
  return parseInt(expiry) // absolute unix
}

// PermitSingle typed data — must match src/signing.rs PermitSingle{...}.
export async function buildApprove(network: string, account: `0x${string}`, budgetUsd: string) {
  const cfg = await apiGet(network, '/v1/system/config')
  const operator = cfg.addresses.operator_hub as `0x${string}`
  const ns = await apiGet(network, `/v1/nonce-state/${account}`)
  const nonceAnchor = BigInt(ns.nonce_anchor ?? '0')
  const nonceBitmap = Number(ns.current_bitmap_index ?? 0)
  const allowanceExpiry = expirySeconds(undefined)
  const budget = BigInt(Math.floor(Number(budgetUsd) * 1e18)).toString()
  const typedData = {
    domain: await domain(network),
    types: {
      PermitSingle: [
        { name: 'account', type: 'address' },
        { name: 'operator', type: 'address' },
        { name: 'budget', type: 'uint96' },
        { name: 'allowanceExpiry', type: 'uint32' },
        { name: 'nonceAnchor', type: 'uint48' },
        { name: 'nonceBitmap', type: 'uint8' },
      ],
    },
    primaryType: 'PermitSingle' as const,
    message: { account, operator, budget: BigInt(budget), allowanceExpiry, nonceAnchor, nonceBitmap },
  }
  return { typedData, operator, budget, allowanceExpiry, nonceAnchor: nonceAnchor.toString(), nonceBitmap }
}

export async function postCallback(callback: string, payload: Record<string, unknown>) {
  await fetch(callback, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })
}
```

- [ ] **Step 2: `web/src/App.tsx`**

```tsx
import { useState } from 'react'
import { useAccount, useConnect, useSignTypedData } from 'wagmi'
import { buildApprove, buildLogin, postCallback, readParams } from './sign'

export function App() {
  const params = readParams()
  const { address, isConnected } = useAccount()
  const { connectors, connect } = useConnect()
  const { signTypedDataAsync } = useSignTypedData()
  const [status, setStatus] = useState('')

  async function authorize() {
    if (!address) return
    setStatus('Building request…')
    try {
      if (params.action === 'approve') {
        const a = await buildApprove(params.network, address, params.budget ?? '0')
        const signature = await signTypedDataAsync(a.typedData as any)
        await postCallback(params.callback, {
          state: params.state, action: 'approve', account: address,
          operator: a.operator, budget: a.budget, allowance_expiry: a.allowanceExpiry,
          nonce_anchor: a.nonceAnchor, nonce_bitmap_index: a.nonceBitmap, signature,
        })
      } else {
        const l = await buildLogin(params.network, address)
        const signature = await signTypedDataAsync(l.typedData as any)
        await postCallback(params.callback, {
          state: params.state, action: 'login', account: address,
          nonce: l.nonce, deadline: l.deadline, signature,
        })
      }
      setStatus('Done — return to your terminal. You can close this tab.')
    } catch (e: any) {
      setStatus(`Error: ${e?.message ?? e}`)
    }
  }

  return (
    <main style={{ fontFamily: 'system-ui', maxWidth: 460, margin: '64px auto', padding: 16 }}>
      <h1>Connect to RISEx CLI</h1>
      <p>Network: <b>{params.network}</b> · Action: <b>{params.action}</b>
        {params.action === 'approve' && <> · Budget: <b>${params.budget}</b></>}</p>
      {!isConnected ? (
        <div>
          {connectors.map((c) => (
            <button key={c.uid} onClick={() => connect({ connector: c })}
              style={{ display: 'block', margin: '8px 0', padding: '10px 14px' }}>
              Connect {c.name}
            </button>
          ))}
        </div>
      ) : (
        <div>
          <p>Connected: {address}</p>
          <button onClick={authorize} style={{ padding: '10px 14px' }}>
            {params.action === 'approve' ? 'Sign approval' : 'Sign login'}
          </button>
        </div>
      )}
      {status && <p style={{ marginTop: 16 }}>{status}</p>}
    </main>
  )
}
```

- [ ] **Step 3: Verify build**

Run: `cd web && npm run build`
Expected: builds clean.

- [ ] **Step 4: Commit**

```bash
git add web/src/
git commit -m "feat(web): wallet connect + EIP-712 sign + callback POST"
```

---

### Task B3: Web README + manual runbook

**Files:** Create `web/README.md`.

- [ ] **Step 1: Write it**

```md
# risex-connect

Wallet-connect bridge for the RISEx CLI (`risex auth connect`). Built with Vite + wagmi + viem;
deployed to https://connect.risescan.io.

## Build & deploy
```sh
cd web && npm install
VITE_WALLETCONNECT_PROJECT_ID=<id> npm run build   # outputs web/dist
# deploy web/dist to connect.risescan.io (served at /cli)
```

## Manual test (needs a wallet)
1. Run a local CLI build: `cargo run -- -n testnet --connect-url http://localhost:5173 auth connect`
   (in another shell: `cd web && npm run dev`).
2. Connect a wallet, sign Login. The terminal should print `Connected as 0x…`.
3. `cargo run -- -n testnet positions` should work without a key.
```

- [ ] **Step 2: Commit**

```bash
git add web/README.md
git commit -m "docs(web): build/deploy + manual runbook"
```

---

## Self-Review

**Spec coverage:** §2 handshake → A2 (bridge) + A3 (connect URL/flow) + B2 (page posts signature). §3 web app (wagmi/viem, login+approve typed data, config, params) → B1, B2. §4 CLI side (`auth connect`, `bridge.rs`, connect-url resolution, browser open, session no-key branch, account persistence) → A1, A2, A3. §5 security (state, 127.0.0.1, single callback, timeout, 0600) → A2, A3. §6 coexistence (key path intact; shared JWT cache) → A1. §7 testing (hermetic bridge/connect tests, session unit test, web manual runbook) → A1, A2, A3, B3.

**Placeholder scan:** No "TBD"/vague steps; every code step is complete. The web `App.tsx` `as any` casts on typed data are deliberate (viem's strict typed-data generics vs. our runtime-built object), not placeholders.

**Type consistency:** `ensure_token(client, account, network_label, Option<&Signer>, verbose)` is used identically in A1's session impl and the trade.rs/auth.rs callers. `Bridge::{start,port,state,await_callback}` and `Callback{action,account,fields}` match between A2 and A3. The callback JSON keys posted by the web app (B2) exactly match the keys A3 reads (`nonce`,`deadline`,`signature` for login; `operator`,`budget`,`allowance_expiry`,`nonce_anchor`,`nonce_bitmap_index`,`signature` for approve) and what A3 forwards to the API. EIP-712 field names/types in B2 match `src/signing.rs` and the Global Constraints.

## Risks

- **EIP-712 parity (web vs. Rust vs. contract).** The page rebuilds the typed data in viem; any mismatch in field order/types/domain makes the contract reject the signature. The real proof is a live testnet `auth connect --approve` succeeding (same caveat as the trading plan). Mitigation: identical field lists are pinned in the Global Constraints and cross-checked in self-review.
- **WalletConnect projectId.** Without `VITE_WALLETCONNECT_PROJECT_ID`, the WC connector is omitted (injected + Coinbase still work). Set it at deploy time to enable WC.
- **Browser↔localhost.** Relies on `127.0.0.1` being a potentially-trustworthy origin (so the HTTPS page may POST to it). Standard and widely used, but corporate proxies/extensions can interfere; the CLI prints the URL and a 180s window as the fallback path.
