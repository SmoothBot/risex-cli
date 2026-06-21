import { useState } from 'react'
import { useAccount, useConnect, useSignTypedData } from 'wagmi'
import { buildApprove, buildLogin, postCallback, readParams } from './sign'

export function App() {
  const params = readParams()
  const { address, isConnected } = useAccount()
  const { connectors, connect } = useConnect()
  const { signTypedDataAsync } = useSignTypedData()
  const [status, setStatus] = useState('')
  const [busy, setBusy] = useState(false)

  async function authorize() {
    if (!address) return
    setBusy(true)
    setStatus('Building request…')
    try {
      if (params.action === 'approve') {
        const a = await buildApprove(params.network, address, params.budget ?? '0', params.expiry)
        setStatus('Sign the approval in your wallet…')
        const signature = await signTypedDataAsync(a.typedData as any)
        await postCallback(params.callback, {
          state: params.state,
          action: 'approve',
          account: address,
          operator: a.operator,
          budget: a.budget,
          allowance_expiry: a.allowanceExpiry,
          nonce_anchor: a.nonceAnchor,
          nonce_bitmap_index: a.nonceBitmap,
          signature,
        })
      } else {
        const l = await buildLogin(params.network, address)
        setStatus('Sign the login in your wallet…')
        const signature = await signTypedDataAsync(l.typedData as any)
        await postCallback(params.callback, {
          state: params.state,
          action: 'login',
          account: address,
          nonce: l.nonce,
          deadline: l.deadline,
          signature,
        })
      }
      setStatus('Done — return to your terminal. You can close this tab.')
    } catch (e: any) {
      setStatus(`Error: ${e?.shortMessage ?? e?.message ?? e}`)
    } finally {
      setBusy(false)
    }
  }

  return (
    <main style={{ fontFamily: 'system-ui, sans-serif', maxWidth: 460, margin: '64px auto', padding: 16 }}>
      <h1>Connect to RISEx CLI</h1>
      <p>
        Network: <b>{params.network}</b> · Action: <b>{params.action}</b>
        {params.action === 'approve' && (
          <>
            {' '}
            · Budget: <b>${params.budget}</b>
          </>
        )}
      </p>
      {!isConnected ? (
        <div>
          {connectors.map((c) => (
            <button
              key={c.uid}
              onClick={() => connect({ connector: c })}
              style={{ display: 'block', margin: '8px 0', padding: '10px 14px', width: '100%' }}
            >
              Connect {c.name}
            </button>
          ))}
        </div>
      ) : (
        <div>
          <p style={{ wordBreak: 'break-all' }}>Connected: {address}</p>
          <button onClick={authorize} disabled={busy} style={{ padding: '10px 14px' }}>
            {params.action === 'approve' ? 'Sign approval' : 'Sign login'}
          </button>
        </div>
      )}
      {status && <p style={{ marginTop: 16 }}>{status}</p>}
    </main>
  )
}
