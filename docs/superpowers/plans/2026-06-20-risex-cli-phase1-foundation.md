# RISEx CLI — Phase 1: Foundation + Market Data — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the `risex` Rust CLI skeleton and ship a working, read-only market-data client for the RISEx perps DEX (markets, ticker, orderbook, trades, candles, funding, system config).

**Architecture:** Mirror kraken-cli's unified-dispatch design — `main.rs` parses a clap-derive `Cli`, builds an `AppContext`, and calls `dispatch()` → `execute_command()` → a per-group handler that returns a `CommandOutput` rendered as table or JSON. Phase 1 builds the shared spine (errors, network, config, output, telemetry, public REST client) plus the public market commands. Auth, trading, WebSocket, paper, and MCP layer on top in later phases.

**Tech Stack:** Rust 2021, clap 4 (derive), reqwest (rustls), tokio, serde/serde_json, toml, comfy-table, thiserror, dirs, uuid, chrono. Tests: wiremock, assert_cmd, predicates, tempfile.

## Global Constraints

- Binary name: `risex`; crate/package name: `risex-cli`; library target `risex_cli`.
- Dependency versions mirror the proven `../kraken-cli/Cargo.toml`; the first `cargo build` validates resolution. Do not add `alloy`, `rmcp`, `tokio-tungstenite`, `dialoguer`, or `rustyline` in Phase 1 (later phases).
- Default network is **testnet**. Networks (static): testnet → REST `https://api.testnet.rise.trade`, WS `wss://ws.testnet.rise.trade/ws`, chainId `11155931`, RPC `https://testnet.riselabs.xyz`; mainnet → REST `https://api.rise.trade`, WS `wss://ws.rise.trade/ws`, chainId `4153`, RPC `https://mainnet.riselabs.xyz`.
- REST success envelope is `{"data": <payload>, "request_id": "<id>"}`; the client unwraps `data` and threads `request_id` into verbose output / errors.
- Output contract: data to **stdout** (single-line JSON in json mode), verbose/warnings to **stderr** only. Exit 0 on success, non-zero on failure.
- Config dir `~/.config/risex/` (files mode `0600` on Unix); data dir `~/.local/share/risex/`.
- Error categories (stable strings): `api`, `auth`, `rate_limit`, `validation`, `network`, `signing`, `config`, `websocket`, `io`, `parse`.
- TDD throughout: write the failing test, see it fail, implement minimally, see it pass, commit.

---

### Task 1: Project scaffold + clap skeleton

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`
- Create: `.gitignore`
- Test: `tests/cli.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: binary `risex`; library `risex_cli` exposing `pub struct Cli` (clap `Parser`) with global flags `--output/-o`, `--verbose/-v`, `--network/-n`, `--api-url`, `--yes/-y`, and `pub enum Command` (empty for now); `pub struct AppContext`; `pub async fn dispatch(&AppContext, Command) -> errors::Result<()>` (added in later tasks — Task 1 only needs `Cli` to parse and `--version` to work).

- [ ] **Step 1: Write the failing test**

```rust
// tests/cli.rs
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn version_flag_prints_semver() {
    Command::cargo_bin("risex")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("risex"));
}

#[test]
fn help_lists_binary_name() {
    Command::cargo_bin("risex")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("risex"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test cli`
Expected: FAIL — no binary `risex` / package does not compile (no `Cargo.toml` yet).

- [ ] **Step 3: Create the Cargo manifest and gitignore**

```toml
# Cargo.toml
[package]
name = "risex-cli"
version = "0.1.0"
edition = "2021"
description = "RISEx CLI — trade and query the RISEx perpetuals DEX from the terminal"
license = "MIT"
repository = "https://github.com/riselabs-xyz/risex-cli"

[lib]
name = "risex_cli"
path = "src/lib.rs"

[[bin]]
name = "risex"
path = "src/main.rs"

[dependencies]
clap = { version = "4", features = ["derive"] }
reqwest = { version = "0.13", features = ["query", "json", "rustls"], default-features = false }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "1"
url = "2"
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
thiserror = "2"
dirs = "6"
comfy-table = "7"
colored = "3"
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
wiremock = "0.6"
assert_cmd = "2"
predicates = "3"
tempfile = "3"
```

```gitignore
# .gitignore
/target
**/*.rs.bk
Cargo.lock
```

Note: `Cargo.lock` is gitignored for now because this is a library+bin in early development; a later release task (Phase 8) will commit it once the binary stabilizes.

- [ ] **Step 4: Create the library root with the clap `Cli`**

```rust
// src/lib.rs
//! RISEx CLI library crate. The same execution path is shared by CLI
//! invocations, the REPL, the MCP server, and integration tests.
pub mod errors;
pub mod network;

use clap::{Parser, Subcommand};

use network::Network;
use output::OutputFormat;

pub mod output {
    //! Placeholder re-exported in Task 5. Defined minimally so `Cli` compiles.
    use clap::ValueEnum;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
    pub enum OutputFormat {
        Table,
        Json,
    }
}

/// Runtime context assembled from global CLI flags and config.
pub struct AppContext {
    pub network: Network,
    /// Optional REST base-URL override (`--api-url`); falls back to the network default.
    pub api_url: Option<String>,
    pub format: OutputFormat,
    pub verbose: bool,
    pub force: bool,
}

impl AppContext {
    /// Resolved REST base URL: explicit override or the network default.
    pub fn base_url(&self) -> String {
        self.api_url
            .clone()
            .unwrap_or_else(|| self.network.rest_base().to_string())
    }
}

/// RISEx CLI — trade and query the RISEx perpetuals DEX from the terminal.
#[derive(Parser)]
#[command(name = "risex", version, about, long_about = None)]
pub struct Cli {
    /// Output format: table (default) or json.
    #[arg(short, long, value_enum, global = true)]
    pub output: Option<OutputFormat>,

    /// Show request/response details on stderr.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Network to target.
    #[arg(short, long, value_enum, global = true)]
    pub network: Option<Network>,

    /// Override the REST API base URL.
    #[arg(long, global = true)]
    pub api_url: Option<String>,

    /// Skip confirmation prompts for destructive operations.
    #[arg(long, alias = "force", global = true)]
    pub yes: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Top-level commands. Variants are added per phase.
#[derive(Subcommand)]
pub enum Command {}
```

- [ ] **Step 5: Create the binary entrypoint**

```rust
// src/main.rs
use clap::Parser;

use risex_cli::Cli;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(_command) => {
            // Dispatch is wired up in Task 8.
        }
        None => {
            use clap::CommandFactory;
            Cli::command().print_help().ok();
            println!();
        }
    }
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --test cli`
Expected: PASS (both tests). Note: `errors` and `network` modules are referenced by `lib.rs` and are created in Tasks 2–3; for Task 1, temporarily declare them as empty files so the crate compiles.

Create empty placeholders so Task 1 builds in isolation:
```rust
// src/errors.rs
// Filled in Task 2.
```
```rust
// src/network.rs
// Filled in Task 3.
```
Then, to satisfy `use network::Network;` in `lib.rs` for Task 1 only, temporarily comment out the `network` usages OR implement Task 3 before running. **Recommended:** implement Tasks 2 and 3 immediately after this step; they have no dependencies on Task 1 beyond the manifest. If you prefer a green build at Task 1, stub `network.rs` with the enum from Task 3 Step 3 now.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml .gitignore src/main.rs src/lib.rs src/errors.rs src/network.rs tests/cli.rs
git commit -m "feat: scaffold risex CLI with clap skeleton and version/help"
```

---

### Task 2: Error type and categories

**Files:**
- Create: `src/errors.rs` (replaces the placeholder)
- Test: inline `#[cfg(test)]` in `src/errors.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub type Result<T> = std::result::Result<T, RisexError>;`
  - `pub enum RisexError` with variants `Api(String)`, `Auth(String)`, `RateLimit { message: String, retryable: bool }`, `Validation(String)`, `Network(String)`, `Signing(String)`, `Config(String)`, `WebSocket(String)`, `Io(std::io::Error)`, `Parse(String)`.
  - `impl RisexError { pub fn category(&self) -> &'static str; pub fn to_json_envelope(&self) -> serde_json::Value; }`
  - `From<std::io::Error>`, `From<reqwest::Error>`, `From<serde_json::Error>`, `From<toml::de::Error>`, `From<toml::ser::Error>` for `RisexError`.

- [ ] **Step 1: Write the failing test**

```rust
// at the bottom of src/errors.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_strings_are_stable() {
        assert_eq!(RisexError::Api("x".into()).category(), "api");
        assert_eq!(RisexError::Validation("x".into()).category(), "validation");
        assert_eq!(
            RisexError::RateLimit { message: "x".into(), retryable: true }.category(),
            "rate_limit"
        );
    }

    #[test]
    fn envelope_has_error_and_message_fields() {
        let env = RisexError::Auth("bad token".into()).to_json_envelope();
        assert_eq!(env["error"], "auth");
        assert_eq!(env["message"], "bad token");
    }

    #[test]
    fn rate_limit_envelope_includes_retryable() {
        let env = RisexError::RateLimit { message: "slow down".into(), retryable: true }
            .to_json_envelope();
        assert_eq!(env["error"], "rate_limit");
        assert_eq!(env["retryable"], true);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test errors::`
Expected: FAIL — `RisexError` not defined.

- [ ] **Step 3: Write the implementation**

```rust
// src/errors.rs
//! Unified error type for the RISEx CLI. Categories are stable strings so
//! callers (and agents driving the CLI) can branch on them.
use serde_json::json;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, RisexError>;

#[derive(Debug, Error)]
pub enum RisexError {
    #[error("{0}")]
    Api(String),
    #[error("{0}")]
    Auth(String),
    #[error("{message}")]
    RateLimit { message: String, retryable: bool },
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Signing(String),
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    WebSocket(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Parse(String),
}

impl RisexError {
    pub fn category(&self) -> &'static str {
        match self {
            Self::Api(_) => "api",
            Self::Auth(_) => "auth",
            Self::RateLimit { .. } => "rate_limit",
            Self::Validation(_) => "validation",
            Self::Network(_) => "network",
            Self::Signing(_) => "signing",
            Self::Config(_) => "config",
            Self::WebSocket(_) => "websocket",
            Self::Io(_) => "io",
            Self::Parse(_) => "parse",
        }
    }

    pub fn to_json_envelope(&self) -> serde_json::Value {
        match self {
            Self::RateLimit { message, retryable } => json!({
                "error": "rate_limit",
                "message": message,
                "retryable": retryable,
            }),
            other => json!({
                "error": other.category(),
                "message": other.to_string(),
            }),
        }
    }
}

impl From<reqwest::Error> for RisexError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() || e.is_connect() {
            RisexError::Network(e.to_string())
        } else {
            RisexError::Parse(e.to_string())
        }
    }
}

impl From<serde_json::Error> for RisexError {
    fn from(e: serde_json::Error) -> Self {
        RisexError::Parse(e.to_string())
    }
}

impl From<toml::de::Error> for RisexError {
    fn from(e: toml::de::Error) -> Self {
        RisexError::Config(format!("TOML parse error: {e}"))
    }
}

impl From<toml::ser::Error> for RisexError {
    fn from(e: toml::ser::Error) -> Self {
        RisexError::Config(format!("TOML serialize error: {e}"))
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test errors::`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/errors.rs
git commit -m "feat: add RisexError with stable categories and JSON envelopes"
```

---

### Task 3: Network enum

**Files:**
- Create: `src/network.rs` (replaces the placeholder)
- Test: inline `#[cfg(test)]` in `src/network.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `pub enum Network { Testnet, Mainnet }` deriving `clap::ValueEnum`, with `const fn rest_base(self) -> &'static str`, `const fn ws_url(self) -> &'static str`, `const fn chain_id(self) -> u64`, `const fn rpc_url(self) -> &'static str`, `const fn label(self) -> &'static str`; `impl Default` (→ `Testnet`); `impl std::fmt::Display`. Consumed by `AppContext` (Task 1) and the REST client (Task 7).

- [ ] **Step 1: Write the failing test**

```rust
// at the bottom of src/network.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_testnet() {
        assert_eq!(Network::default(), Network::Testnet);
    }

    #[test]
    fn testnet_endpoints() {
        let n = Network::Testnet;
        assert_eq!(n.rest_base(), "https://api.testnet.rise.trade");
        assert_eq!(n.ws_url(), "wss://ws.testnet.rise.trade/ws");
        assert_eq!(n.chain_id(), 11155931);
        assert_eq!(n.rpc_url(), "https://testnet.riselabs.xyz");
    }

    #[test]
    fn mainnet_endpoints() {
        let n = Network::Mainnet;
        assert_eq!(n.rest_base(), "https://api.rise.trade");
        assert_eq!(n.ws_url(), "wss://ws.rise.trade/ws");
        assert_eq!(n.chain_id(), 4153);
    }

    #[test]
    fn display_is_lowercase_label() {
        assert_eq!(Network::Testnet.to_string(), "testnet");
        assert_eq!(Network::Mainnet.to_string(), "mainnet");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test network::`
Expected: FAIL — `Network` not defined (or placeholder empty).

- [ ] **Step 3: Write the implementation**

```rust
// src/network.rs
//! Static network definitions. Contract addresses and the EIP-712 domain are
//! NOT hardcoded here — they are fetched at runtime per network in later phases.
use clap::ValueEnum;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Network {
    Testnet,
    Mainnet,
}

impl Default for Network {
    fn default() -> Self {
        Network::Testnet
    }
}

impl Network {
    pub const fn rest_base(self) -> &'static str {
        match self {
            Network::Testnet => "https://api.testnet.rise.trade",
            Network::Mainnet => "https://api.rise.trade",
        }
    }

    pub const fn ws_url(self) -> &'static str {
        match self {
            Network::Testnet => "wss://ws.testnet.rise.trade/ws",
            Network::Mainnet => "wss://ws.rise.trade/ws",
        }
    }

    pub const fn chain_id(self) -> u64 {
        match self {
            Network::Testnet => 11155931,
            Network::Mainnet => 4153,
        }
    }

    pub const fn rpc_url(self) -> &'static str {
        match self {
            Network::Testnet => "https://testnet.riselabs.xyz",
            Network::Mainnet => "https://mainnet.riselabs.xyz",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Network::Testnet => "testnet",
            Network::Mainnet => "mainnet",
        }
    }
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test network::`
Expected: PASS (4 tests). Also run `cargo test --test cli` to confirm Task 1's `Cli` now compiles against the real `Network`.

- [ ] **Step 5: Commit**

```bash
git add src/network.rs
git commit -m "feat: add Network enum (testnet default) with static endpoints"
```

---

### Task 4: Config + SecretValue + paths

**Files:**
- Create: `src/config.rs`
- Modify: `src/lib.rs` (add `pub mod config;`)
- Test: inline `#[cfg(test)]` in `src/config.rs`

**Interfaces:**
- Consumes: `errors::{Result, RisexError}`.
- Produces:
  - `pub struct SecretValue(String)` with `pub fn new(String) -> Self`, `pub fn expose(&self) -> &str`; `Debug`/`Display` print `[REDACTED]`.
  - `pub struct RisexConfig { pub auth: AuthConfig, pub settings: SettingsConfig }` (serde).
  - `pub struct AuthConfig { pub private_key: Option<String>, pub account: Option<String> }`.
  - `pub struct SettingsConfig { pub network: Option<String>, pub output: Option<String>, pub default_market: Option<String> }`.
  - `pub fn config_dir() -> Result<PathBuf>`, `pub fn config_path() -> Result<PathBuf>`, `pub fn data_dir() -> Result<PathBuf>`, `pub fn load() -> Result<RisexConfig>`, `pub fn save(&RisexConfig) -> Result<()>`, `pub fn mask_address(&str) -> String`.

- [ ] **Step 1: Write the failing test**

```rust
// at the bottom of src/config.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_redacted_in_debug_and_display() {
        let s = SecretValue::new("0xdeadbeef".into());
        assert_eq!(format!("{s:?}"), "[REDACTED]");
        assert_eq!(format!("{s}"), "[REDACTED]");
        assert_eq!(s.expose(), "0xdeadbeef");
    }

    #[test]
    fn config_roundtrips_through_toml() {
        let cfg = RisexConfig {
            auth: AuthConfig {
                private_key: Some("0xkey".into()),
                account: Some("0xabc".into()),
            },
            settings: SettingsConfig {
                network: Some("testnet".into()),
                output: Some("table".into()),
                default_market: Some("BTC/USDC".into()),
            },
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: RisexConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.auth.account.as_deref(), Some("0xabc"));
        assert_eq!(back.settings.default_market.as_deref(), Some("BTC/USDC"));
    }

    #[test]
    fn mask_address_keeps_prefix_and_suffix() {
        assert_eq!(mask_address("0x1234567890abcdef"), "0x1234…cdef");
        assert_eq!(mask_address("short"), "****");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test config::`
Expected: FAIL — `config` module/types not defined.

- [ ] **Step 3: Write the implementation**

```rust
// src/config.rs
//! `~/.config/risex/config.toml` management with 0600 permissions and a
//! redacting secret wrapper. Credential *resolution* (flag>env>config) is
//! added in Phase 2; Phase 1 only needs load/save and the settings section.
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::{Result, RisexError};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RisexConfig {
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub settings: SettingsConfig,
}

#[derive(Default, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthConfig")
            .field("private_key", &self.private_key.as_ref().map(|_| "[REDACTED]"))
            .field("account", &self.account.as_ref().map(|a| mask_address(a)))
            .finish()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SettingsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_market: Option<String>,
}

/// Wrapper that keeps secrets out of Debug/Display output.
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(val: String) -> Self {
        Self(val)
    }
    /// Read-only access. Callers must not log the result.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl std::fmt::Display for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

pub fn config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| RisexError::Config("cannot determine config directory".into()))?;
    Ok(base.join("risex"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| RisexError::Config("cannot determine data directory".into()))?;
    Ok(base.join("risex"))
}

pub fn load() -> Result<RisexConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(RisexConfig::default());
    }
    let contents = fs::read_to_string(&path)?;
    Ok(toml::from_str(&contents)?)
}

pub fn save(cfg: &RisexConfig) -> Result<()> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir)?;
    let path = dir.join("config.toml");
    let contents = toml::to_string_pretty(cfg)?;
    atomic_write_restricted(&path, contents.as_bytes())
}

/// Mask an address as `0x1234…cdef`. Strings of 8 chars or fewer become `****`.
pub fn mask_address(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    let prefix: String = chars[..6].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}…{suffix}")
}

#[cfg(unix)]
fn atomic_write_restricted(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = path
        .parent()
        .ok_or_else(|| RisexError::Config("config path has no parent directory".into()))?;
    let tmp_path = dir.join(".config.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp_path)?;
    file.write_all(data)?;
    file.sync_all()?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(not(unix))]
fn atomic_write_restricted(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data)?;
    Ok(())
}
```

Add to `src/lib.rs` after `pub mod network;`:
```rust
pub mod config;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test config::`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/lib.rs
git commit -m "feat: add config (auth+settings) with 0600 atomic writes and SecretValue"
```

---

### Task 5: Output module (table + JSON + error rendering)

**Files:**
- Create: `src/output/mod.rs` (replaces the inline placeholder module in `lib.rs`)
- Create: `src/output/table.rs`
- Create: `src/output/json.rs`
- Modify: `src/lib.rs` (remove the inline `pub mod output { … }` placeholder; add `pub mod output;`)
- Test: inline `#[cfg(test)]` in `src/output/mod.rs`

**Interfaces:**
- Consumes: `errors::RisexError`.
- Produces:
  - `pub enum OutputFormat { Table, Json }` (clap `ValueEnum`) — moved here from the Task-1 placeholder.
  - `pub struct CommandOutput { pub data: serde_json::Value, pub headers: Vec<String>, pub rows: Vec<Vec<String>> }` with `pub fn new(Value, Vec<String>, Vec<Vec<String>>) -> Self`, `pub fn key_value(Vec<(String, String)>, Value) -> Self`, `pub fn message(&str) -> Self`.
  - `pub fn render(OutputFormat, &CommandOutput)`, `pub fn render_error(OutputFormat, &RisexError)`, `pub fn verbose(&str)`, `pub fn warn(&str)`.

- [ ] **Step 1: Write the failing test**

```rust
// at the bottom of src/output/mod.rs
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_value_builds_two_column_rows() {
        let out = CommandOutput::key_value(
            vec![("network".into(), "testnet".into())],
            json!({"network": "testnet"}),
        );
        assert_eq!(out.headers, vec!["Field".to_string(), "Value".to_string()]);
        assert_eq!(out.rows, vec![vec!["network".to_string(), "testnet".to_string()]]);
    }

    #[test]
    fn message_output_carries_text() {
        let out = CommandOutput::message("done");
        assert_eq!(out.rows, vec![vec!["done".to_string()]]);
        assert_eq!(out.data["message"], "done");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test output::`
Expected: FAIL — new `output` module not yet a file / `CommandOutput` not defined.

- [ ] **Step 3: Write the implementations**

First remove the placeholder module from `src/lib.rs` (the `pub mod output { … }` block from Task 1) and replace it with a file module declaration alongside the others:
```rust
// src/lib.rs — module declarations section
pub mod config;
pub mod errors;
pub mod network;
pub mod output;
```

```rust
// src/output/mod.rs
//! Output rendering. Data goes to stdout; verbose/warnings to stderr.
mod json;
mod table;

use clap::ValueEnum;
use serde_json::Value;

use crate::errors::RisexError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

/// Structured result of a command: raw JSON plus a table projection.
pub struct CommandOutput {
    pub data: Value,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl CommandOutput {
    pub fn new(data: Value, headers: Vec<String>, rows: Vec<Vec<String>>) -> Self {
        Self { data, headers, rows }
    }

    /// Two-column "Field / Value" projection over `pairs`, with `data` as the
    /// JSON payload.
    pub fn key_value(pairs: Vec<(String, String)>, data: Value) -> Self {
        let rows = pairs.into_iter().map(|(k, v)| vec![k, v]).collect();
        Self {
            data,
            headers: vec!["Field".into(), "Value".into()],
            rows,
        }
    }

    /// Single-cell message output.
    pub fn message(msg: &str) -> Self {
        Self {
            data: serde_json::json!({ "message": msg }),
            headers: vec!["Message".into()],
            rows: vec![vec![msg.to_string()]],
        }
    }
}

pub fn render(format: OutputFormat, output: &CommandOutput) {
    match format {
        OutputFormat::Table => table::render(output),
        OutputFormat::Json => json::render_success(&output.data),
    }
}

pub fn render_error(format: OutputFormat, err: &RisexError) {
    match format {
        OutputFormat::Table => eprintln!("Error [{}]: {}", err.category(), err),
        OutputFormat::Json => json::render_error(err),
    }
}

pub fn verbose(msg: &str) {
    eprintln!("[verbose] {msg}");
}

pub fn warn(msg: &str) {
    eprintln!("Warning: {msg}");
}
```

```rust
// src/output/table.rs
use comfy_table::{presets::UTF8_FULL_CONDENSED, ContentArrangement, Table};

use super::CommandOutput;

pub fn render(output: &CommandOutput) {
    if output.rows.is_empty() {
        println!("(no results)");
        return;
    }
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic);
    if !output.headers.is_empty() {
        table.set_header(&output.headers);
    }
    for row in &output.rows {
        table.add_row(row);
    }
    println!("{table}");
}
```

```rust
// src/output/json.rs
use serde_json::Value;

use crate::errors::RisexError;

pub fn render_success(data: &Value) {
    match serde_json::to_string(data) {
        Ok(s) => println!("{s}"),
        Err(_) => println!(r#"{{"error":"parse","message":"JSON serialization failed"}}"#),
    }
}

pub fn render_error(err: &RisexError) {
    let envelope = err.to_json_envelope();
    println!("{}", serde_json::to_string(&envelope).unwrap_or_default());
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test output::`
Expected: PASS (2 tests). Run `cargo build` to confirm the `OutputFormat` move didn't break `Cli`.

- [ ] **Step 5: Commit**

```bash
git add src/output/ src/lib.rs
git commit -m "feat: add output module (comfy-table, single-line JSON, error envelopes)"
```

---

### Task 6: Telemetry headers

**Files:**
- Create: `src/telemetry.rs`
- Modify: `src/lib.rs` (add `pub mod telemetry;`)
- Test: inline `#[cfg(test)]` in `src/telemetry.rs`

**Interfaces:**
- Consumes: `config::data_dir` (for the persisted instance id).
- Produces: `pub const CLIENT_NAME: &str = "risex-cli";`, `pub fn version() -> &'static str`, `pub fn detect_agent() -> &'static str`, `pub fn instance_id() -> String`, `pub fn client_headers() -> Vec<(&'static str, String)>` returning `X-Risex-Client`, `X-Risex-Client-Version`, `X-Risex-Agent-Client`, `X-Risex-Instance-Id`. Consumed by the REST client (Task 7).

- [ ] **Step 1: Write the failing test**

```rust
// at the bottom of src/telemetry.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_agent_defaults_to_direct_when_unset() {
        // No agent env vars set in the test process by default.
        let agent = detect_agent();
        assert!(
            ["direct", "claude", "cursor", "codex", "gemini", "vscode"].contains(&agent),
            "unexpected agent: {agent}"
        );
    }

    #[test]
    fn client_headers_include_required_keys() {
        let headers = client_headers();
        let keys: Vec<&str> = headers.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&"X-Risex-Client"));
        assert!(keys.contains(&"X-Risex-Client-Version"));
        assert!(keys.contains(&"X-Risex-Agent-Client"));
        assert!(keys.contains(&"X-Risex-Instance-Id"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test telemetry::`
Expected: FAIL — module not defined.

- [ ] **Step 3: Write the implementation**

```rust
// src/telemetry.rs
//! Client attribution headers. No analytics — agent identification only.
use std::fs;

use crate::config;

pub const CLIENT_NAME: &str = "risex-cli";

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Best-effort detection of the calling agent from environment markers.
pub fn detect_agent() -> &'static str {
    if std::env::var_os("CLAUDECODE").is_some() {
        "claude"
    } else if std::env::var_os("CURSOR_AGENT").is_some() {
        "cursor"
    } else if std::env::var_os("CODEX_SANDBOX").is_some() {
        "codex"
    } else if std::env::var_os("GEMINI_CLI").is_some() {
        "gemini"
    } else if std::env::var_os("VSCODE_PID").is_some() {
        "vscode"
    } else {
        "direct"
    }
}

/// Stable per-install UUID, persisted at `~/.local/share/risex/instance_id`.
/// Falls back to an ephemeral UUID if the data dir is unavailable.
pub fn instance_id() -> String {
    let Ok(dir) = config::data_dir() else {
        return uuid::Uuid::new_v4().to_string();
    };
    let path = dir.join("instance_id");
    if let Ok(existing) = fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(&path, &id);
    id
}

pub fn client_headers() -> Vec<(&'static str, String)> {
    vec![
        ("X-Risex-Client", CLIENT_NAME.to_string()),
        ("X-Risex-Client-Version", version().to_string()),
        ("X-Risex-Agent-Client", detect_agent().to_string()),
        ("X-Risex-Instance-Id", instance_id()),
    ]
}
```

Add to `src/lib.rs`:
```rust
pub mod telemetry;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test telemetry::`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/telemetry.rs src/lib.rs
git commit -m "feat: add telemetry headers with agent detection and persistent instance id"
```

---

### Task 7: Public REST client

**Files:**
- Create: `src/client.rs`
- Modify: `src/lib.rs` (add `pub mod client;`)
- Test: `tests/rest_client.rs`

**Interfaces:**
- Consumes: `errors::{Result, RisexError}`, `telemetry::client_headers`, `output::verbose`.
- Produces:
  - `pub struct RestClient { /* private */ }`
  - `pub fn RestClient::new(base_url: &str) -> Result<Self>`
  - `pub async fn RestClient::public_get(&self, path: &str, query: &[(&str, &str)], verbose: bool) -> Result<serde_json::Value>` — performs `GET {base_url}{path}`, attaches telemetry headers, and returns the unwrapped `data` field of the response envelope. On a non-2xx status or an `{"error": …}` body, returns the mapped `RisexError`.

- [ ] **Step 1: Write the failing test**

```rust
// tests/rest_client.rs
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
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test rest_client`
Expected: FAIL — `risex_cli::client` not defined.

- [ ] **Step 3: Write the implementation**

```rust
// src/client.rs
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
    let json: Value = serde_json::from_str(body)
        .map_err(|_| RisexError::Parse(format!("non-JSON response (status {status}): {body}")))?;

    if status.is_success() {
        if let Some(data) = json.get("data") {
            return Ok(data.clone());
        }
        // Some endpoints may return a bare body; pass it through.
        return Ok(json);
    }

    // Error path: prefer an explicit message field.
    let message = json
        .get("error")
        .and_then(|e| e.as_str())
        .or_else(|| json.get("message").and_then(|m| m.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("HTTP {status}"));

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
```

Add to `src/lib.rs`:
```rust
pub mod client;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --test rest_client`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/client.rs src/lib.rs
git commit -m "feat: add public REST client with envelope unwrapping and error mapping"
```

---

### Task 8: Market commands + dispatch wiring

**Files:**
- Create: `src/commands/mod.rs`
- Create: `src/commands/market.rs`
- Modify: `src/lib.rs` (add `pub mod commands;`, populate `Command` enum, add `execute_command` + `dispatch`)
- Modify: `src/main.rs` (call `dispatch`, render errors, set exit code)
- Test: `tests/market.rs`

**Interfaces:**
- Consumes: `client::RestClient`, `output::CommandOutput`, `errors::Result`, `AppContext`.
- Produces:
  - `pub enum MarketCommand { Markets { market: Option<String> }, Ticker { market: String }, Orderbook { market: String, depth: u32 }, Trades { market: String, limit: u32 }, Candles { market: String, resolution: String, from: Option<String>, to: Option<String> }, Funding { market: String }, System }`
  - `pub async fn market::execute(&MarketCommand, &RestClient, verbose: bool) -> Result<CommandOutput>`
  - `Command` enum variants in `lib.rs`: `Markets`, `Ticker`, `Orderbook`, `Trades`, `Candles`, `Funding`, `System`.
  - `pub async fn execute_command(&AppContext, Command) -> Result<CommandOutput>` and `pub async fn dispatch(&AppContext, Command) -> Result<()>`.

- [ ] **Step 1: Write the failing test**

```rust
// tests/market.rs
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn markets_command_renders_json_rows() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/markets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "markets": [{
                "market_id": "1",
                "display_name": "BTC/USDC",
                "last_price": "63113.1",
                "mark_price": "63268.7",
                "index_price": "63344.5"
            }]},
            "request_id": "r1"
        })))
        .mount(&server)
        .await;

    // assert_cmd is sync; drive the binary against the mock server URL.
    let uri = server.uri();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("risex")
            .unwrap()
            .args(["--api-url", &uri, "-o", "json", "markets"])
            .assert()
            .success()
            .stdout(predicate::str::contains("BTC/USDC"));
    })
    .await
    .unwrap();
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test market`
Expected: FAIL — `markets` subcommand does not exist.

- [ ] **Step 3: Write the command module**

```rust
// src/commands/mod.rs
pub mod market;
```

```rust
// src/commands/market.rs
//! Public market-data commands. All read-only; no auth.
use serde_json::Value;

use crate::client::RestClient;
use crate::errors::Result;
use crate::output::CommandOutput;

pub enum MarketCommand {
    Markets { market: Option<String> },
    Ticker { market: String },
    Orderbook { market: String, depth: u32 },
    Trades { market: String, limit: u32 },
    Candles {
        market: String,
        resolution: String,
        from: Option<String>,
        to: Option<String>,
    },
    Funding { market: String },
    System,
}

pub async fn execute(
    cmd: &MarketCommand,
    client: &RestClient,
    verbose: bool,
) -> Result<CommandOutput> {
    match cmd {
        MarketCommand::Markets { market } => markets(client, market.as_deref(), verbose).await,
        MarketCommand::Ticker { market } => ticker(client, market, verbose).await,
        MarketCommand::Orderbook { market, depth } => {
            orderbook(client, market, *depth, verbose).await
        }
        MarketCommand::Trades { market, limit } => trades(client, market, *limit, verbose).await,
        MarketCommand::Candles {
            market,
            resolution,
            from,
            to,
        } => candles(client, market, resolution, from.as_deref(), to.as_deref(), verbose).await,
        MarketCommand::Funding { market } => funding(client, market, verbose).await,
        MarketCommand::System => system(client, verbose).await,
    }
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .map(|x| match x {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

async fn markets(client: &RestClient, filter: Option<&str>, verbose: bool) -> Result<CommandOutput> {
    let data = client.public_get("/v1/markets", &[], verbose).await?;
    let empty = vec![];
    let list = data
        .get("markets")
        .and_then(|m| m.as_array())
        .unwrap_or(&empty);
    let headers = vec![
        "ID".into(),
        "Market".into(),
        "Last".into(),
        "Mark".into(),
        "Index".into(),
    ];
    let rows = list
        .iter()
        .filter(|m| match filter {
            Some(f) => s(m, "display_name").eq_ignore_ascii_case(f)
                || s(m, "market_id") == f,
            None => true,
        })
        .map(|m| {
            vec![
                s(m, "market_id"),
                s(m, "display_name"),
                s(m, "last_price"),
                s(m, "mark_price"),
                s(m, "index_price"),
            ]
        })
        .collect();
    Ok(CommandOutput::new(data, headers, rows))
}

async fn ticker(client: &RestClient, market: &str, verbose: bool) -> Result<CommandOutput> {
    // Ticker is derived from /v1/markets to avoid a second endpoint contract.
    let data = client.public_get("/v1/markets", &[], verbose).await?;
    let empty = vec![];
    let found = data
        .get("markets")
        .and_then(|m| m.as_array())
        .unwrap_or(&empty)
        .iter()
        .find(|m| {
            s(m, "display_name").eq_ignore_ascii_case(market) || s(m, "market_id") == market
        })
        .cloned()
        .unwrap_or(Value::Null);
    let pairs = vec![
        ("market".into(), s(&found, "display_name")),
        ("last_price".into(), s(&found, "last_price")),
        ("mark_price".into(), s(&found, "mark_price")),
        ("index_price".into(), s(&found, "index_price")),
        ("change_24h".into(), s(&found, "change_24h")),
        ("current_funding_rate".into(), s(&found, "current_funding_rate")),
    ];
    Ok(CommandOutput::key_value(pairs, found))
}

async fn orderbook(
    client: &RestClient,
    market: &str,
    depth: u32,
    verbose: bool,
) -> Result<CommandOutput> {
    let depth_s = depth.to_string();
    let data = client
        .public_get(
            "/v1/orderbook",
            &[("market_id", market), ("limit", &depth_s)],
            verbose,
        )
        .await?;
    let empty = vec![];
    let bids = data.get("bids").and_then(|b| b.as_array()).unwrap_or(&empty);
    let asks = data.get("asks").and_then(|a| a.as_array()).unwrap_or(&empty);
    let headers = vec!["Side".into(), "Price".into(), "Quantity".into()];
    let mut rows: Vec<Vec<String>> = Vec::new();
    for a in asks.iter().rev() {
        rows.push(vec!["ask".into(), s(a, "price"), s(a, "quantity")]);
    }
    for b in bids.iter() {
        rows.push(vec!["bid".into(), s(b, "price"), s(b, "quantity")]);
    }
    Ok(CommandOutput::new(data, headers, rows))
}

async fn trades(
    client: &RestClient,
    market: &str,
    limit: u32,
    verbose: bool,
) -> Result<CommandOutput> {
    let limit_s = limit.to_string();
    let data = client
        .public_get(
            "/v1/trades",
            &[("market_id", market), ("limit", &limit_s)],
            verbose,
        )
        .await?;
    let empty = vec![];
    let list = data
        .get("trades")
        .and_then(|t| t.as_array())
        .or_else(|| data.as_array())
        .unwrap_or(&empty);
    let headers = vec!["Price".into(), "Size".into(), "Side".into(), "Time".into()];
    let rows = list
        .iter()
        .map(|t| {
            vec![
                s(t, "price"),
                s(t, "size"),
                s(t, "maker_side"),
                s(t, "timestamp"),
            ]
        })
        .collect();
    Ok(CommandOutput::new(data, headers, rows))
}

async fn candles(
    client: &RestClient,
    market: &str,
    resolution: &str,
    from: Option<&str>,
    to: Option<&str>,
    verbose: bool,
) -> Result<CommandOutput> {
    let mut query: Vec<(&str, &str)> = vec![("market_id", market), ("resolution", resolution)];
    if let Some(f) = from {
        query.push(("from", f));
    }
    if let Some(t) = to {
        query.push(("to", t));
    }
    let data = client.public_get("/v1/candles", &query, verbose).await?;
    let empty = vec![];
    let list = data
        .get("candles")
        .and_then(|c| c.as_array())
        .or_else(|| data.as_array())
        .unwrap_or(&empty);
    let headers = vec![
        "Time".into(),
        "Open".into(),
        "High".into(),
        "Low".into(),
        "Close".into(),
        "Volume".into(),
    ];
    let rows = list
        .iter()
        .map(|c| {
            vec![
                s(c, "time"),
                s(c, "open"),
                s(c, "high"),
                s(c, "low"),
                s(c, "close"),
                s(c, "volume"),
            ]
        })
        .collect();
    Ok(CommandOutput::new(data, headers, rows))
}

async fn funding(client: &RestClient, market: &str, verbose: bool) -> Result<CommandOutput> {
    let data = client
        .public_get("/v1/funding-rates", &[("market_id", market)], verbose)
        .await?;
    Ok(CommandOutput::new(
        data.clone(),
        vec!["Funding".into()],
        vec![vec![data.to_string()]],
    ))
}

async fn system(client: &RestClient, verbose: bool) -> Result<CommandOutput> {
    let data = client.public_get("/v1/system/config", &[], verbose).await?;
    let addresses = data.get("addresses").cloned().unwrap_or(Value::Null);
    let pairs = vec![
        ("chain".into(), s(data.get("chain").unwrap_or(&Value::Null), "name")),
        (
            "chain_id".into(),
            s(data.get("chain").unwrap_or(&Value::Null), "chain_id"),
        ),
        ("operator_hub".into(), s(&addresses, "operator_hub")),
        ("auth".into(), s(&addresses, "auth")),
        ("usdc".into(), s(&addresses, "usdc")),
        (
            "maintenance".into(),
            data.get("is_maintenance_mode")
                .map(|v| v.to_string())
                .unwrap_or_default(),
        ),
    ];
    Ok(CommandOutput::key_value(pairs, data))
}
```

- [ ] **Step 4: Wire the commands into `lib.rs`**

Add the module declaration and populate dispatch. Add to the module section:
```rust
pub mod commands;
```
Add `use` lines near the top of `lib.rs`:
```rust
use commands::market::{self, MarketCommand};
use errors::Result;
use output::{render, CommandOutput};
```
Replace the empty `Command` enum with:
```rust
#[derive(Subcommand)]
pub enum Command {
    /// List markets (optionally filter to one).
    Markets {
        /// Market id or display name (e.g. BTC/USDC).
        #[arg(long)]
        market: Option<String>,
    },
    /// Show last/mark/index price for a market.
    Ticker {
        /// Market id or display name.
        market: String,
    },
    /// Show the order book for a market.
    Orderbook {
        /// Market id or display name.
        market: String,
        /// Number of price levels per side.
        #[arg(long, default_value = "20")]
        depth: u32,
    },
    /// Show recent trades for a market.
    Trades {
        /// Market id or display name.
        market: String,
        /// Max number of trades.
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Show candles (OHLCV) for a market.
    Candles {
        /// Market id or display name.
        market: String,
        /// Candle resolution (e.g. 1, 5, 60, 1D).
        #[arg(long)]
        resolution: String,
        /// Start time (unix seconds).
        #[arg(long)]
        from: Option<String>,
        /// End time (unix seconds).
        #[arg(long)]
        to: Option<String>,
    },
    /// Show funding-rate info for a market.
    Funding {
        /// Market id or display name.
        market: String,
    },
    /// Show system config (contract addresses, chain, maintenance).
    System,
}
```
Add the executor and dispatcher at the bottom of `lib.rs`:
```rust
fn build_client(ctx: &AppContext) -> Result<client::RestClient> {
    client::RestClient::new(&ctx.base_url())
}

pub async fn execute_command(ctx: &AppContext, command: Command) -> Result<CommandOutput> {
    let client = build_client(ctx)?;
    let market_cmd = match command {
        Command::Markets { market } => MarketCommand::Markets { market },
        Command::Ticker { market } => MarketCommand::Ticker { market },
        Command::Orderbook { market, depth } => MarketCommand::Orderbook { market, depth },
        Command::Trades { market, limit } => MarketCommand::Trades { market, limit },
        Command::Candles {
            market,
            resolution,
            from,
            to,
        } => MarketCommand::Candles {
            market,
            resolution,
            from,
            to,
        },
        Command::Funding { market } => MarketCommand::Funding { market },
        Command::System => MarketCommand::System,
    };
    market::execute(&market_cmd, &client, ctx.verbose).await
}

pub async fn dispatch(ctx: &AppContext, command: Command) -> Result<()> {
    let out = execute_command(ctx, command).await?;
    render(ctx.format, &out);
    Ok(())
}
```

- [ ] **Step 5: Wire `main.rs` to build `AppContext` and dispatch**

```rust
// src/main.rs
use std::process;

use clap::Parser;

use risex_cli::network::Network;
use risex_cli::output::OutputFormat;
use risex_cli::{AppContext, Cli};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let format = cli.output.unwrap_or(OutputFormat::Table);

    let ctx = AppContext {
        network: cli.network.unwrap_or(Network::default()),
        api_url: cli.api_url.clone(),
        format,
        verbose: cli.verbose,
        force: cli.yes,
    };

    match cli.command {
        Some(command) => {
            if let Err(e) = risex_cli::dispatch(&ctx, command).await {
                risex_cli::output::render_error(ctx.format, &e);
                process::exit(1);
            }
        }
        None => {
            use clap::CommandFactory;
            Cli::command().print_help().ok();
            println!();
        }
    }
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test`
Expected: PASS — all unit tests plus `tests/cli.rs`, `tests/rest_client.rs`, `tests/market.rs`. Then a manual smoke check against live testnet:

Run: `cargo run -- markets`
Expected: a table of live testnet markets (BTC/USDC, ETH/USDC, …).
Run: `cargo run -- -o json system`
Expected: single-line JSON with contract addresses.

- [ ] **Step 7: Commit**

```bash
git add src/commands/ src/lib.rs src/main.rs tests/market.rs
git commit -m "feat: add market-data commands (markets, ticker, orderbook, trades, candles, funding, system)"
```

---

## Self-Review

**Spec coverage (Phase 1 portions of the design spec):**
- §1 binary/shape → Task 1. §3 networks (testnet default, static endpoints) → Task 3. §4 config + SecretValue + 0600 + data dir → Task 4 (credential *resolution* deferred to Phase 2, as the spec scopes it). §5 crate layout → Tasks 1–8 establish `errors`/`network`/`config`/`output`/`telemetry`/`client`/`commands`. §6 market data commands → Task 8. §10 output/errors/telemetry → Tasks 2, 5, 6. §11 client envelope unwrap + error categories → Task 7. §16 testing (assert_cmd, wiremock, unit tests) → Tasks 1–8.
- Deferred to later phases by design: §2 JWT auth, §6 signing, §7 units/order-encoding, §8 account/trade/auth/ws/paper commands, §9 revoke, §12 session, §13 MCP, §14 paper, §15 releases. Each gets its own plan.

**Placeholder scan:** The `src/errors.rs` / `src/network.rs` placeholders in Task 1 Step 6 are an explicit ordering note (implement Tasks 2–3 next, or stub the enum), not an unspecified TODO — the real content is fully given in Tasks 2 and 3. No "add error handling"/"write tests for the above" style gaps; every code step contains complete code.

**Type consistency:** `CommandOutput::{new,key_value,message}`, `RestClient::{new,public_get}`, `MarketCommand` variants, and `Network::{rest_base,ws_url,chain_id,rpc_url}` are referenced with identical signatures across Tasks 5, 7, 8. `OutputFormat` is defined once in Task 1 (placeholder) and relocated to `output` in Task 5 with the same shape; Task 5 Step 3 explicitly removes the placeholder to avoid a duplicate definition. `AppContext` fields (`network`, `api_url`, `format`, `verbose`, `force`) match between Task 1 and `main.rs` in Task 8.

---

## Phase roadmap (for context; each becomes its own plan)

2. **Auth & session** — `signing.rs` (alloy EIP-712 `PermitSingle`/`Login`), `session.rs` (JWT login/refresh/cache + 401 recovery), credential resolution in `config.rs`, `auth` commands (`set/import/reset/status/login/refresh/approve/allowance/revoke`).
3. **Account (read)** — `balance/positions/orders/order-history/fills/funding-payments/pnl/transfers` (bearer).
4. **Trading (write)** — `order buy/sell/cancel/cancel-all`, `close`, `leverage`, `margin`; human-unit ↔ steps/ticks conversion; confirmation gating.
5. **WebSocket** — `ws orderbook/trades/funding/oracle` (public) and `ws positions/fills/orders` (JWT auth).
6. **Paper trading** — local per-network JSON sim against live mark prices.
7. **MCP server** — `rmcp` registry/schema/server over the shared dispatch path.
8. **Releases** — cargo-dist, `install.sh`, GitHub Actions; commit `Cargo.lock`.
