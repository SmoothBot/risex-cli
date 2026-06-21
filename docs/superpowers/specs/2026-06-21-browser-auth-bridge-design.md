# RISEx CLI — Browser Auth Bridge — Design Spec

**Date:** 2026-06-21
**Status:** Approved (design) — pending written-spec review
**Depends on:** Phase 2+4 trading (JWT auth/session) — already shipped.

## 1. Purpose

Let users authenticate `risex` **without pasting a private key**. Instead, `risex auth connect`
opens a hosted wallet-connect page; the user connects their wallet and signs the EIP-712 payload
there; the page returns only the **signature** to a one-shot localhost server; the CLI completes the
RISEx API call and stores only the JWT. The account key never leaves the user's wallet, and the CLI
never holds it.

This reuses the entire JWT auth/session/trading path already built. It adds a hosted web app (source
in `web/`, deployed by RISEx) and a CLI-side callback bridge.

## 2. Handshake & data flow

```
risex auth connect -n testnet
  ├─ CLI starts a one-shot HTTP server on http://127.0.0.1:<rand-port>
  ├─ CLI opens the browser to:
  │     https://connect.risescan.io/cli?network=testnet&callback=http://127.0.0.1:PORT&state=<csrf>&action=login
  │
  [hosted page — web/]
  ├─ wagmi: Connect Wallet (injected / WalletConnect / Coinbase …)
  ├─ read connected account
  ├─ fetch nonce + EIP-712 domain from the RISEx API (page's own origin; CORS fine)
  ├─ build the Login EIP-712 typed data → wallet signs (eth_signTypedData_v4)
  ├─ POST http://127.0.0.1:PORT/callback  { state, action, account, nonce, deadline, signature }
  │
  └─ CLI verifies state → POST /v1/auth/login → stores JWT + account → 200 → server shuts down
```

`127.0.0.1` is a *potentially-trustworthy* origin, so the HTTPS page is permitted to POST to the
local HTTP server without a mixed-content block. Only a signature crosses the callback; tokens never
transit the browser.

For `--approve`, `action=approve` is used, `budget`/`expiry` ride in the URL, the page builds and
signs `PermitSingle` (fetching operator from `/v1/system/config` and `(nonce_anchor, bitmap_index)`
from `/v1/nonce-state/{account}`), and the CLI completes `POST /v1/auth/approve-single`.

## 3. The web app (`web/`)

- **Stack:** Vite + React + **wagmi** + **viem**. wagmi connectors provide injected wallets,
  WalletConnect, and Coinbase from one connect UI.
- **Responsibilities:**
  - Connect wallet; read account.
  - `action=login`: `GET /v1/auth/nonce?account=…` + `GET /v1/auth/eip712-domain`; build `Login`
    typed data; sign; POST `{state, action:"login", account, nonce, deadline, signature}` to callback.
  - `action=approve`: `GET /v1/system/config` (operator_hub) + domain + `GET /v1/nonce-state/{account}`;
    build `PermitSingle` from the URL's `budget` (USD → WAD) and `expiry`; sign; POST
    `{state, action:"approve", account, operator, budget, allowance_expiry, nonce_anchor,
    nonce_bitmap_index, signature}` to callback.
  - Show success/failure; instruct the user to return to the terminal.
- **Config (build-time, owned by RISEx):** `VITE_WALLETCONNECT_PROJECT_ID`; the two RISE chains
  (mainnet `4153`, testnet `11155931`) with RPC URLs (`https://mainnet.riselabs.xyz`,
  `https://testnet.riselabs.xyz`); a `network → API base` map (mainnet `https://api.rise.trade`,
  testnet `https://api.testnet.rise.trade`). URL params read at runtime: `network`, `callback`,
  `state`, `action`, `budget`, `expiry`.
- **Deployment:** hosted by RISEx at `https://connect.risescan.io`. This repo holds only the source.
  The EIP-712 typed data the page builds must match exactly what the CLI's `signing.rs` builds and
  what the contracts expect (same domain, struct fields, types).

## 4. The CLI side

- **New subcommand `risex auth connect [--approve --budget <usd> --expiry <dur>]`** (in
  `src/commands/auth.rs`): resolve the account is *not* required up front (the wallet provides it);
  start the local callback server; open the browser; await one signed callback (hard timeout ~180s);
  complete `/v1/auth/login` (and `/v1/auth/approve-single` when `--approve`); persist JWT + account;
  shut down. If the browser can't auto-open, print the URL.
- **New module `src/bridge.rs`:** the one-shot localhost server. Responsibilities: bind `127.0.0.1`
  on a random port; serve a tiny `200 OK` page and accept `POST /callback`; enforce the `state`
  token; set permissive CORS for the connect origin (and handle `OPTIONS` preflight); return the
  parsed callback payload to the caller; single successful callback then stop; timeout. Implemented
  with a minimal embedded HTTP server (`tiny_http`).
- **Connect URL resolution:** `--connect-url` flag > `RISEX_CONNECT_URL` env > default
  `https://connect.risescan.io`. The `network` query param is derived from `ctx.network`.
- **Browser open:** best-effort via the `open` crate; on failure, print the URL for manual paste.
- **`session.rs` change:** `ensure_token` keeps its cached→refresh path. New branch: if the refresh
  token is dead/absent **and** no private key is configured, return
  `RisexError::Auth("session expired — run `risex auth connect`")` rather than attempting to sign.
  Result: after one `auth connect`, all trading commands work for the 7-day refresh window with no
  browser; only a lapsed refresh requires reconnecting.
- **Account persistence:** on successful connect, write `config.auth.account` (no key) so read
  commands (`positions`, `balance`) work without a key.

## 5. Security model

- The private key stays in the user's wallet; the CLI stores only `account` + the JWT (refresh token),
  mode `0600`, exactly as the key-based path does.
- The `state` nonce binds the callback to this CLI run; the server binds `127.0.0.1` only, uses a
  random port, accepts a single successful callback, then exits, with a hard timeout.
- Signatures are nonce- and deadline-bound (one-shot, ~5 min), so a captured callback is replay-useless.
- For `--approve`, the wallet's signing UI shows the real `PermitSingle` message (operator, budget,
  expiry), so the user sees exactly what they authorize.

## 6. Coexistence with the key path

`auth import --private-key` remains for CI/headless use (no browser). `auth connect` is the
recommended interactive path. Both feed the same JWT cache, so `order`, `positions`, `close`,
`leverage`, `margin`, `balance` are unchanged regardless of how the session was established.

## 7. Testing

- **CLI / bridge (hermetic, in the Rust suite):** a test plays the role of the browser — it POSTs a
  canned signed payload to the local callback while the RISEx API is a wiremock server — and asserts
  that login completes and the JWT is cached. Cover: happy path (login), `--approve` path,
  `state` mismatch rejected, timeout returns a clear error.
- **`session.rs`:** unit test the no-key/expired-refresh branch returns the reconnect error.
- **Web app:** light component tests where practical; the wallet/extension seam (connect + sign) is
  verified manually. Not part of the Rust suite. A short manual runbook is added to the web app README.

## 8. Out of scope (future)

- WalletConnect/hardware nuances beyond what wagmi provides by default.
- The generated-signer / per-order-permit "agent key" model (the more autonomous alternative
  considered and deferred).
- `auth connect` auto-triggering approve: approve is always explicit (`--approve`) so an on-chain
  budget grant never happens implicitly.

## 9. File map

```
web/                         # Vite + React + wagmi + viem source (deployed to connect.risescan.io)
  package.json, vite.config.ts, index.html
  src/ (App, connectors/chains config, sign flows)
  README.md (build/deploy + manual test runbook)
src/bridge.rs                # one-shot localhost callback server (tiny_http)
src/commands/auth.rs         # + Connect subcommand
src/session.rs               # no-key / expired-refresh -> reconnect error
src/lib.rs, src/main.rs      # --connect-url flag/env wiring
```
