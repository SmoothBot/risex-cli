# risex-connect

Wallet-connect bridge for the RISEx CLI (`risex auth connect`). Built with Vite + wagmi + viem and
deployed to https://connect.risescan.io. The CLI opens this page with `?network=…&callback=…&state=…&action=…`;
the page connects a wallet, signs the EIP-712 payload, and POSTs **only the signature** back to the
CLI's one-shot `127.0.0.1` callback.

## Build & deploy

```sh
cd web
npm install
VITE_WALLETCONNECT_PROJECT_ID=<id> npm run build   # outputs web/dist
# serve web/dist at https://connect.risescan.io so the app is reachable at /cli
```

Without `VITE_WALLETCONNECT_PROJECT_ID` the build still works — it ships the injected (MetaMask/Rabby)
and Coinbase connectors; set the id to add WalletConnect.

## EIP-712 parity (important)

The typed data in `src/sign.ts` MUST match the Rust signer (`../src/signing.rs`) and the contracts:

- `Login { account: address, nonce: uint256, deadline: uint32 }`
- `PermitSingle { account: address, operator: address, budget: uint96, allowanceExpiry: uint32, nonceAnchor: uint48, nonceBitmap: uint8 }`
- Domain `{ name, version, chainId, verifyingContract }` from `GET /v1/auth/eip712-domain`.

If any field name, type, or order drifts, the contract rejects the signature.

## Manual test (needs a browser wallet)

1. Terminal 1: `cd web && npm run dev` (serves on http://localhost:5173).
2. Terminal 2: `cargo run -- -n testnet --connect-url http://localhost:5173 auth connect`
3. The browser opens; connect a wallet and sign **Login**. The terminal should print `Connected as 0x…`.
4. `cargo run -- -n testnet positions` should now work with no private key.
5. For approval: `cargo run -- -n testnet --connect-url http://localhost:5173 auth connect --approve --budget 1000`,
   sign **PermitSingle**, and confirm the terminal prints a `transaction_hash`.
