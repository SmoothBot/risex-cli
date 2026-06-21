# risex-cli

Trade and query the [RISEx](https://rise.trade) perpetuals DEX from your terminal.

`risex` is a fast, scriptable CLI for RISEx — a fully on-chain CLOB perps DEX on RISE Chain. Phase 1 ships read-only market data; trading, account, WebSocket streaming, paper trading, and an MCP server are on the roadmap.

## Install

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/SmoothBot/risex-cli/releases/latest/download/risex-cli-installer.sh | sh
```

Or build from source:

```sh
cargo install --path .
```

## Usage

```sh
risex markets                      # list all markets
risex ticker btc                   # last/mark/index price (by ticker, name, pair, or id)
risex orderbook btc                # depth-10 book, auto-aggregated, with cumulative depth + spread
risex orderbook eth --no-agg       # raw tick-level book
risex orderbook btc -a 50 --amount # $50 buckets, base-size columns
risex trades btc --limit 20
risex candles btc --resolution 60
risex funding btc
risex system                       # contract addresses / chain info
```

Global flags: `-o/--output table|json`, `-v/--verbose`, `-n/--network mainnet|testnet`, `--api-url <url>`.

### Networks & environment

Defaults to **mainnet**. Use `-n testnet` for the RISEx testnet.

Resolution precedence is **flag > env var > default** for both:

| Setting | Flag | Env var |
|---|---|---|
| Network | `-n/--network` | `RISEX_NETWORK` |
| REST base URL | `--api-url` | `RISEX_API_URL` |

Idempotent GET requests automatically retry on transient network errors and 5xx responses (exponential backoff, up to 3 attempts).

## Connect a wallet (no private key)

The recommended way to authenticate — your key never touches the CLI:

```sh
risex -n testnet auth connect                            # opens browser; sign Login in your wallet
risex -n testnet auth connect --approve --budget 1000    # one-time; sign ApproveSingle in your wallet
risex -n testnet order buy btc 0.001 --type market       # trades for the 7-day session, no browser
```

`auth connect` starts a one-shot `127.0.0.1` server, opens `connect.risescan.io`, and your wallet
signs the EIP-712 payload there — only the **signature** returns to the CLI, which completes the API
call and stores just the JWT. Override the page with `--connect-url` / `RISEX_CONNECT_URL`. Once
connected, trading works with no key for the refresh window; when it lapses, run `auth connect` again.

## Trading (JWT auth, with a stored key)

RISEx uses a JWT session model: one on-chain `ApproveSingle` grants an operator a USD
budget, then each session is a single `Login` signature — no per-order signing.

**First-time setup on testnet** (recommended before mainnet):

```sh
risex auth import --private-key 0xYOURKEY     # stores key at ~/.config/risex/config.toml (0600)
risex -n testnet auth approve --budget 1000   # one-time: signs PermitSingle, grants $1000 budget
risex -n testnet auth status                  # allowance: active
```

**Open / inspect / close a position:**

```sh
risex -n testnet order buy btc 0.001 --type market     # open a long
risex -n testnet positions                             # see it
risex -n testnet order sell btc 0.001 --price 70000 --post-only   # resting limit short
risex -n testnet order cancel btc <order-id>
risex -n testnet close btc                             # reduce-only market close
risex -n testnet balance
risex -n testnet leverage btc 10
risex -n testnet margin btc isolated
```

Credentials resolve **flag > env > config**: `--private-key` / `RISEX_PRIVATE_KEY`,
`--account` / `RISEX_ACCOUNT` (account is derived from the key when omitted). The private key
is never logged. Write commands prompt for confirmation (skip with `-y`) and print a red
warning on **mainnet**.

## Output

Human-readable tables by default; `-o json` emits single-line JSON for scripting. Errors are categorized (`api`, `auth`, `rate_limit`, `validation`, `network`, …) and, in JSON mode, returned as structured envelopes.

## License

MIT
