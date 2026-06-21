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
