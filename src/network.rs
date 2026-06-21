//! Static network definitions. Contract addresses and the EIP-712 domain are
//! NOT hardcoded here — they are fetched at runtime per network in later phases.
use clap::ValueEnum;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum Network {
    Testnet,
    #[default]
    Mainnet,
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

impl std::str::FromStr for Network {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mainnet" | "main" => Ok(Network::Mainnet),
            "testnet" | "test" => Ok(Network::Testnet),
            other => Err(format!(
                "unknown network '{other}' (expected 'mainnet' or 'testnet')"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_mainnet() {
        assert_eq!(Network::default(), Network::Mainnet);
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
