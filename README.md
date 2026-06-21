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

### Networks

Defaults to **mainnet**. Use `-n testnet` for the RISEx testnet.

## Output

Human-readable tables by default; `-o json` emits single-line JSON for scripting. Errors are categorized (`api`, `auth`, `rate_limit`, `validation`, `network`, …) and, in JSON mode, returned as structured envelopes.

## License

MIT
