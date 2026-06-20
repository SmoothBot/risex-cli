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
