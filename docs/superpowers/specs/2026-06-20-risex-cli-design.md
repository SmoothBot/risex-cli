# RISEx CLI — Design Spec

**Date:** 2026-06-20
**Status:** Approved (design) — pending written-spec review
**Binary:** `risex`

## 1. Purpose & scope

A Rust CLI for trading on **RISEx** — a fully on-chain CLOB perpetuals DEX on RISE Chain.
The CLI is structurally modeled on [`kraken-cli`](../../../../kraken-cli) (unified dispatch
path, clap-derive command tree, `CommandOutput` rendering, embedded MCP server, REPL, paper
trading, curl-installable releases) but its client/auth core is RISEx-specific.

**Key architectural fact:** with the JWT auth model the CLI is a **pure REST + WebSocket
client plus an EIP-712 signing helper**. There is **no chain RPC, no gas, and no transaction
broadcasting** in the core flow — the backend submits the on-chain `ApproveSingle` for you and
returns the tx hash. The single exception is `auth revoke` (§9), which sends one on-chain tx.

### v1 includes
- Market data (public), account/positions (read), trading (write), WebSocket streaming
- JWT auth/session management
- Leverage & margin-mode updates
- Embedded MCP server, interactive REPL shell, paper trading
- curl-installable releases (cargo-dist)

### Explicitly out of scope for v1
- Liquidation simulation in paper trading (documented simplification)
- Legacy per-order EIP-712 permit path (JWT path only)
- Programmatic signer registration (`RegisterSigner`) — not needed for JWT flow

## 2. Authentication model (JWT)

Per the *RISEx API Trader Auth (JWT) Guide*:

1. **One-time `ApproveSingle` (on-chain, backend-submitted).** Sign `PermitSingle` EIP-712 with
   the account key → `POST /v1/auth/approve-single`. Grants the OperatorHub a notional USD budget
   and returns the first `access_token` + `refresh_token`.
2. **Per-session `Login` (off-chain).** `GET /v1/auth/nonce?account=…` → sign `Login` EIP-712 with
   the **account key** → `POST /v1/auth/login` → `access_token` (TTL 15m) + `refresh_token` (TTL 7d).
3. **Refresh.** `POST /v1/auth/refresh` rotates the pair when the access token nears expiry. On
   failure (rotated/expired/revoked), fall back to a full `Login`.
4. **Trade.** `Authorization: Bearer <access_token>` on every write call — **no per-order signature,
   no bitmap nonce, no `permit` field** in the body.

Order execution consumes `size × price` (notional USD, WAD) from the on-chain allowance budget.
When the budget runs low or `allowance_expiry` passes, re-run `ApproveSingle`.

## 3. Networks

`Network` enum carries only static base/ws/chainId values. **Contract addresses and the EIP-712
domain are fetched at runtime** per network (`GET /v1/system/config`, `GET /v1/auth/eip712-domain`)
and cached for the process — they differ per environment (confirmed: testnet `verifying_contract`
= `0x6DA86F486b5E6536358F5b122dBe184522CA0eE3`, distinct from staging).

| Network | REST base | WebSocket | chainId | RPC (revoke only) |
|---|---|---|---|---|
| **testnet** (default) | `https://api.testnet.rise.trade` | `wss://ws.testnet.rise.trade/ws` | 11155931 | `https://testnet.riselabs.xyz` |
| **mainnet** | `https://api.rise.trade` | `wss://ws.rise.trade/ws` | 4153 | `https://mainnet.riselabs.xyz` *(assumed; confirm)* |

Selection precedence: `--network/-n` flag > `RISEX_NETWORK` env > config `network` > default (`testnet`).

REST response envelope is `{"data": …, "request_id": …}`. The client unwraps `data` on success and
surfaces `request_id` in verbose output and error envelopes.

## 4. Credentials & config (kraken-cli model)

Config file `~/.config/risex/config.toml`, written atomically with mode `0600` (Unix):

```toml
[auth]
private_key = "0x…"   # account key — signs PermitSingle + Login
account     = "0x…"   # optional; derived from private_key if omitted

[settings]
network        = "testnet"
output         = "table"
default_market = "BTC/USDC"
```

- **Resolution: flag > env > config** — `--private-key` / `RISEX_PRIVATE_KEY`; `--account` /
  `RISEX_ACCOUNT`. Empty-string env values normalized to absent (plugin-host safety).
- **Safer input paths:** `--private-key-stdin`, `--private-key-file`; no-echo `dialoguer` prompt in
  `auth set`.
- **`SecretValue` wrapper** redacts the private key in `Debug`/`Display`; the account address is
  shown masked (last-4).
- **JWT tokens cached separately** at `~/.local/share/risex/session-<network>-<account>.json` (0600):
  `access_token`, `refresh_token`, `access_expiry`, `refresh_expiry`. Lets subsequent commands trade
  without re-signing `Login`. The private key is **never** written to this file.

## 5. Crate layout

```
src/
  main.rs            # parse Cli, build AppContext, dispatch
  lib.rs             # AppContext, Cli, dispatch(), execute_command()  (shared by CLI/REPL/MCP/tests)
  network.rs         # Network enum; runtime SystemConfig + Eip712Domain cache
  config.rs          # config.toml, SecretValue, credential resolution (mirrors kraken)
  signing.rs         # alloy EIP-712: PermitSingle, Login; address-from-key; v-fix
  session.rs         # JWT lifecycle: login / refresh / auto-refresh / 401 recovery; token cache
  client.rs          # reqwest REST: public_get, private_get/post (bearer), unauth_post (signed); retry + rate limit
  errors.rs          # RisexError category enum
  telemetry.rs       # agent detection + headers + instance id
  output/{mod,json,table}.rs
  commands/{mod,market,account,trade,auth,ws,paper,system,utility}.rs
  mcp/{mod,registry,schema,server}.rs
  shell.rs           # REPL
  paper.rs           # paper-trading state
```

### Unified dispatch
`main.rs` → `Cli::parse()` → build `AppContext` → `dispatch()` → `execute_command()` → handler.
Same code path serves CLI, REPL, MCP server, and integration tests (no subprocess overhead).

### `AppContext`
Holds: `network`, resolved REST/WS base URLs (+ `--api-url` override), `OutputFormat`, `verbose`,
`force` (`-y`), resolved credentials (lazy), `mcp_mode`. Runtime `SystemConfig`/`Eip712Domain` are
fetched lazily and cached on first need.

## 6. Signing (`signing.rs`)

`alloy` crate: `alloy-signer-local` (`PrivateKeySigner`, address derivation), `alloy-sol-types` /
`alloy-dyn-abi` for EIP-712 typed data. Two typed structs only:

- `PermitSingle { account: address, operator: address, budget: uint96, allowanceExpiry: uint32,
  nonceAnchor: uint48, nonceBitmap: uint8 }`
- `Login { account: address, nonce: uint256, deadline: uint32 }`

Domain (`name`, `version`, `chainId`, `verifyingContract`) fetched at runtime. Output is a 65-byte
hex signature (`0x…`) with `v ∈ {27,28}` (replicating `fixSignatureV`). Unit-testable against the
EIP-712 struct digests.

## 7. Order encoding & units

**Human units everywhere.** Users pass decimal sizes (`0.01`) and prices (`63000`); the CLI converts
to integer `size_steps` / `price_ticks` using `market.config.step_size` / `step_price`, and formats
API responses back into decimals for display. Market order ⇒ `price_ticks = 0`, `time_in_force = IOC`.

Enum mappings (from API): `side` 0=Buy/Long, 1=Sell/Short; `order_type` 0=Market, 1=Limit;
`time_in_force` 0=GTC, 1=GTT, 2=FOK, 3=IOC; `stp_mode` 0=ExpireMaker, 1=ExpireTaker, 2=ExpireBoth,
3=None; `margin_mode` 0=Cross, 1=Isolated.

## 8. Command surface

Global flags: `-o/--output table|json`, `-v/--verbose`, `-n/--network <mainnet|testnet>`,
`--private-key[-stdin|-file]`, `--account`, `--api-url`, `-y/--yes`.

```
Market data (public):
  markets [--market M]            orderbook <M> [--depth N]     trades <M> [--limit N]
  candles <M> --resolution R [--from --to]    funding <M>       ticker <M>
  system [config|status]

Account (bearer):
  balance        positions [--market M]      orders [--market M]    order-history [--market --limit]
  fills [--market --limit]    funding-payments [--limit]    pnl     transfers [--limit]

Trade (bearer; dangerous → confirm unless -y):
  order buy  <M> <size> [--type limit|market --price P --tif gtc|gtt|fok|ioc --post-only --reduce-only --stp MODE --client-id ID]
  order sell <M> <size> [...same flags...]
  order cancel <M> <order-id>          order cancel-all [--market M]
  close <M>                            (reduce-only market close of full position)
  leverage <M> <x>                     margin <M> cross|isolated

Auth / session:
  auth set | import | reset            auth status         auth login | refresh
  auth approve --budget <usd> [--expiry <dur>]            auth allowance
  auth revoke                          (lockdownAllowance — on-chain tx via RPC; see §9)

Streaming:
  ws orderbook|trades|funding|oracle <M>                 ws positions|fills|orders   (bearer)

Paper (local sim, no auth):
  paper init [--balance 10000]         paper buy|sell <M> <size> [--price --type]
  paper positions|orders|status        paper close <M>     paper reset

Utility:
  shell        setup (wizard)          mcp [-s groups] [--allow-dangerous]
  completions <shell>                  version
```

## 9. `auth revoke` (the one on-chain action)

`lockdownAllowance(operator)` must be sent on-chain by the account (it is the emergency off-switch
for JWT trading). This is the only command requiring chain interaction. Implemented with an
`alloy-provider` against the network RPC (testnet `https://testnet.riselabs.xyz`; mainnet assumed
`https://mainnet.riselabs.xyz`, to confirm). Requires the private key to sign and broadcast; gas paid
by the account. Gated behind confirmation unless `-y`.

## 10. Output, errors, telemetry

- **Output:** `CommandOutput { data, headers, rows }` → `comfy-table` (UTF8 preset, dynamic width)
  for `table`, single-line JSON for `json`. Verbose/warnings to **stderr only**; data to **stdout**.
- **Errors:** unified `RisexError` with stable categories — `api`, `auth`, `rate_limit`,
  `validation`, `network`, `signing`, `config`, `websocket`, `io`, `parse`. JSON error envelope
  `{"error":"<category>","message":…,"request_id":…[,"retryable","suggestion","docs_url"]}`. Exit
  code 0 = success, non-zero = failure.
- **Telemetry:** headers `X-Risex-Client: risex-cli`, `X-Risex-Client-Version`, `X-Risex-Agent-Client`
  (detected: claude/cursor/codex/gemini/vscode/direct), `X-Risex-Instance-Id` (UUID persisted at
  `~/.config/risex/instance_id`, 0600). No analytics; agent attribution only.

## 11. HTTP client (`client.rs`)

`reqwest` with rustls. Methods: `public_get`, `private_get`/`private_post` (inject Bearer via
`session.rs`), `unauth_post` (carries EIP-712 signature, e.g. approve/login/refresh/nonce).
- **Rate limiting:** client-side token bucket mirroring the API (500 req / 10s).
- **Retry:** idempotent GETs retry on transient/5xx with exponential backoff; POST mutations fail
  fast (may have been processed server-side).
- **401 handling:** a bearer call that returns 401 triggers one transparent re-login via `session.rs`,
  then retries once.
- Maps non-2xx bodies and `{"error"/"message"}` fields to `RisexError` categories; attaches
  `request_id`.

## 12. Session manager (`session.rs`)

`ensure_access_token(network, account)`:
1. Load cached tokens. If `access_token` valid (> ~30s from expiry), use it.
2. Else if `refresh_token` valid → `POST /v1/auth/refresh`, persist rotated pair.
3. Else full `Login`: `GET /v1/auth/nonce` → sign `Login` → `POST /v1/auth/login`, persist.

Token expiry derived from `expires_in` + receipt time (no JWT-decode dependency required).

## 13. MCP server (`mcp/`)

Same registry/schema/server pattern as kraken-cli (`rmcp` over stdio, same dispatch path). Service
groups: `market`, `account`, `trade` (dangerous), `auth` (dangerous), `paper`, `system`; `ws`
excluded. `--allow-dangerous` gates write tools (off by default). `risex mcp [-s market,account,paper]`.

## 14. Paper trading (`paper.rs`)

Local JSON state per network at `~/.local/share/risex/paper-<network>.json`. Fills against live
`mark_price` from `/v1/markets`. Perps-aware: tracks position size/side/entry/leverage and unrealized
PnL. **No liquidation engine in v1** (documented simplification). Commands mirror real trading
(`buy`/`sell`/`close`/`positions`/`orders`/`status`/`reset`).

## 15. Releases & install

Mirror kraken-cli: **cargo-dist** (`dist-workspace.toml` + `[profile.dist]` in `Cargo.toml`), GitHub
Actions release workflow, generated `install.sh` so
`curl -LsSf https://github.com/<org>/risex-cli/releases/latest/download/install.sh | sh` works.
Targets: macOS arm64/x64, Linux x64/arm64. Agent plugin manifests optional in a later iteration.

## 16. Testing

- `assert_cmd` + `predicates` for CLI behavior; `wiremock` for REST mocking.
- Unit tests: EIP-712 digests, size↔steps / price↔ticks conversion, token-expiry logic, network
  resolution, credential precedence, secret redaction.
- Optional `#[ignore]` testnet smoke test driven by an env-provided funded key
  (`RISEX_TEST_PRIVATE_KEY`).

## 17. Dependencies (initial)

clap, reqwest (rustls), tokio, serde, serde_json, toml, url, chrono, tokio-tungstenite, rustls,
futures-util, anyhow, thiserror, dirs, comfy-table, dialoguer, rustyline, colored, indicatif, rmcp,
uuid — plus `alloy` (`signer-local`, `sol-types`, `dyn-abi`, `primitives`, and `provider`+`rpc` for
`auth revoke`). Dev: wiremock, assert_cmd, predicates, tempfile.

## 18. Open items to confirm during implementation

1. **Mainnet RPC host** — assumed `https://mainnet.riselabs.xyz`; confirm before shipping `auth revoke`
   on mainnet.
2. **Leverage / margin-mode endpoint paths** — confirmed they exist as bearer REST endpoints; exact
   paths/payloads to be read from the swagger.
3. **Error response body shape** for non-2xx — handle defensively; refine category mapping once a real
   sample is captured.
```
