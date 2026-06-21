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
    name: d.name as string,
    version: d.version as string,
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
export async function buildApprove(
  network: string,
  account: `0x${string}`,
  budgetUsd: string,
  expiry?: string,
) {
  const cfg = await apiGet(network, '/v1/system/config')
  const operator = cfg.addresses.operator_hub as `0x${string}`
  const ns = await apiGet(network, `/v1/nonce-state/${account}`)
  const nonceAnchor = BigInt(ns.nonce_anchor ?? '0')
  const nonceBitmap = Number(ns.current_bitmap_index ?? 0)
  const allowanceExpiry = expirySeconds(expiry)
  const budget = BigInt(Math.floor(Number(budgetUsd) * 1e18))
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
    message: { account, operator, budget, allowanceExpiry, nonceAnchor, nonceBitmap },
  }
  return {
    typedData,
    operator,
    budget: budget.toString(),
    allowanceExpiry,
    nonceAnchor: nonceAnchor.toString(),
    nonceBitmap,
  }
}

export async function postCallback(callback: string, payload: Record<string, unknown>) {
  await fetch(callback, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })
}
