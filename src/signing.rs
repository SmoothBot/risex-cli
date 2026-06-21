//! EIP-712 signing for the JWT auth flow. Only two typed structs are needed:
//! `PermitSingle` (one-time ApproveSingle) and `Login` (per session).
use alloy::primitives::{Address, U256};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use alloy::sol;
use alloy::sol_types::{eip712_domain, SolStruct};

use crate::errors::{Result, RisexError};

sol! {
    #[allow(missing_docs)]
    struct PermitSingle {
        address account;
        address operator;
        uint96 budget;
        uint32 allowanceExpiry;
        uint48 nonceAnchor;
        uint8 nonceBitmap;
    }

    #[allow(missing_docs)]
    struct Login {
        address account;
        uint256 nonce;
        uint32 deadline;
    }
}

/// Runtime EIP-712 domain (fetched from the API per network).
pub struct Eip712Domain {
    pub name: String,
    pub version: String,
    pub chain_id: u64,
    pub verifying_contract: String,
}

fn parse_addr(s: &str) -> Result<Address> {
    s.parse::<Address>()
        .map_err(|e| RisexError::Signing(format!("invalid address '{s}': {e}")))
}

fn build_domain(d: &Eip712Domain) -> Result<alloy::sol_types::Eip712Domain> {
    Ok(eip712_domain! {
        name: d.name.clone(),
        version: d.version.clone(),
        chain_id: d.chain_id,
        verifying_contract: parse_addr(&d.verifying_contract)?,
    })
}

pub struct Signer {
    inner: PrivateKeySigner,
}

impl Signer {
    pub fn from_key(private_key: &str) -> Result<Self> {
        let key = private_key.trim();
        let key = key.strip_prefix("0x").unwrap_or(key);
        let inner = key
            .parse::<PrivateKeySigner>()
            .map_err(|e| RisexError::Signing(format!("invalid private key: {e}")))?;
        Ok(Self { inner })
    }

    pub fn address(&self) -> String {
        self.inner.address().to_checksum(None)
    }

    fn finalize(&self, hash: alloy::primitives::B256) -> Result<String> {
        let sig = self
            .inner
            .sign_hash_sync(&hash)
            .map_err(|e| RisexError::Signing(format!("signing failed: {e}")))?;
        let mut bytes = sig.as_bytes().to_vec(); // [r(32) | s(32) | v(1)]
        if bytes.len() == 65 && bytes[64] < 27 {
            bytes[64] += 27; // normalize y-parity 0/1 -> 27/28
        }
        Ok(format!("0x{}", hex::encode(bytes)))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn sign_permit_single(
        &self,
        d: &Eip712Domain,
        account: &str,
        operator: &str,
        budget: u128,
        allowance_expiry: u32,
        nonce_anchor: u64,
        nonce_bitmap: u8,
    ) -> Result<String> {
        let domain = build_domain(d)?;
        let msg = PermitSingle {
            account: parse_addr(account)?,
            operator: parse_addr(operator)?,
            budget: budget
                .try_into()
                .map_err(|_| RisexError::Signing("budget overflow".into()))?,
            allowanceExpiry: allowance_expiry,
            nonceAnchor: U256::from(nonce_anchor).to(),
            nonceBitmap: nonce_bitmap,
        };
        self.finalize(msg.eip712_signing_hash(&domain))
    }

    pub fn sign_login(
        &self,
        d: &Eip712Domain,
        account: &str,
        nonce_hex: &str,
        deadline: u32,
    ) -> Result<String> {
        let domain = build_domain(d)?;
        let nonce = U256::from_str_radix(nonce_hex.trim_start_matches("0x"), 16)
            .map_err(|e| RisexError::Signing(format!("invalid nonce '{nonce_hex}': {e}")))?;
        let msg = Login {
            account: parse_addr(account)?,
            nonce,
            deadline,
        };
        self.finalize(msg.eip712_signing_hash(&domain))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Well-known Hardhat account #0.
    const KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const ADDR: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    fn domain() -> Eip712Domain {
        Eip712Domain {
            name: "RISEx".into(),
            version: "1".into(),
            chain_id: 11155931,
            verifying_contract: "0x6DA86F486b5E6536358F5b122dBe184522CA0eE3".into(),
        }
    }

    #[test]
    fn derives_checksummed_address() {
        let s = Signer::from_key(KEY).unwrap();
        assert_eq!(s.address(), ADDR);
    }

    #[test]
    fn from_key_accepts_bare_hex() {
        let bare = KEY.trim_start_matches("0x");
        assert_eq!(Signer::from_key(bare).unwrap().address(), ADDR);
    }

    #[test]
    fn login_sig_is_65_bytes_with_valid_v() {
        let s = Signer::from_key(KEY).unwrap();
        let sig = s
            .sign_login(
                &domain(),
                ADDR,
                "0x23c6560f9a08ad3e2fab7b75ca6c36417c3242799b241f7706bf0e7f15c075a7",
                1778573048,
            )
            .unwrap();
        assert!(sig.starts_with("0x"));
        let bytes = hex::decode(sig.trim_start_matches("0x")).unwrap();
        assert_eq!(bytes.len(), 65);
        assert!(
            bytes[64] == 27 || bytes[64] == 28,
            "v must be 27/28, got {}",
            bytes[64]
        );
    }

    #[test]
    fn permit_sig_is_deterministic() {
        let s = Signer::from_key(KEY).unwrap();
        let a = s
            .sign_permit_single(&domain(), ADDR, ADDR, 1_000_000_000_000_000_000_000, 1781164860, 0, 1)
            .unwrap();
        let b = s
            .sign_permit_single(&domain(), ADDR, ADDR, 1_000_000_000_000_000_000_000, 1781164860, 0, 1)
            .unwrap();
        assert_eq!(a, b);
    }
}
