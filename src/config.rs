//! `~/.config/risex/config.toml` management with 0600 permissions and a
//! redacting secret wrapper. Credential *resolution* (flag>env>config) is
//! added in Phase 2; Phase 1 only needs load/save and the settings section.
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::{Result, RisexError};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RisexConfig {
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub settings: SettingsConfig,
}

#[derive(Default, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthConfig")
            .field("private_key", &self.private_key.as_ref().map(|_| "[REDACTED]"))
            .field("account", &self.account.as_ref().map(|a| mask_address(a)))
            .finish()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SettingsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_market: Option<String>,
}

/// Wrapper that keeps secrets out of Debug/Display output.
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(val: String) -> Self {
        Self(val)
    }
    /// Read-only access. Callers must not log the result.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl std::fmt::Display for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

pub fn config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| RisexError::Config("cannot determine config directory".into()))?;
    Ok(base.join("risex"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir()
        .ok_or_else(|| RisexError::Config("cannot determine data directory".into()))?;
    Ok(base.join("risex"))
}

pub fn load() -> Result<RisexConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(RisexConfig::default());
    }
    let contents = fs::read_to_string(&path)?;
    Ok(toml::from_str(&contents)?)
}

pub fn save(cfg: &RisexConfig) -> Result<()> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir)?;
    let path = dir.join("config.toml");
    let contents = toml::to_string_pretty(cfg)?;
    atomic_write_restricted(&path, contents.as_bytes())
}

/// Mask an address as `0x1234…cdef`. Strings of 8 chars or fewer become `****`.
pub fn mask_address(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    let prefix: String = chars[..6].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}…{suffix}")
}

#[cfg(unix)]
fn atomic_write_restricted(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let dir = path
        .parent()
        .ok_or_else(|| RisexError::Config("config path has no parent directory".into()))?;
    let tmp_path = dir.join(".config.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp_path)?;
    file.write_all(data)?;
    file.sync_all()?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(not(unix))]
fn atomic_write_restricted(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data)?;
    Ok(())
}

use crate::signing::Signer;

pub struct Credentials {
    pub private_key: SecretValue,
    pub account: String,
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

/// Resolve trading credentials: flag > env > config. The account address is
/// derived from the private key when not explicitly provided.
pub fn resolve_credentials(
    flag_key: Option<&str>,
    flag_account: Option<&str>,
) -> Result<Credentials> {
    let key = flag_key
        .map(|s| s.to_string())
        .or_else(|| env_nonempty("RISEX_PRIVATE_KEY"))
        .or_else(|| load().ok().and_then(|c| c.auth.private_key));
    let key = key.ok_or_else(|| {
        RisexError::Auth(
            "No private key found. Use `risex auth import --private-key 0x…`, set \
             RISEX_PRIVATE_KEY, or pass --private-key."
                .into(),
        )
    })?;

    // Validate + derive address.
    let signer = Signer::from_key(&key)?;
    let derived = signer.address();

    let account = flag_account
        .map(|s| s.to_string())
        .or_else(|| env_nonempty("RISEX_ACCOUNT"))
        .or_else(|| load().ok().and_then(|c| c.auth.account))
        .unwrap_or(derived);

    Ok(Credentials {
        private_key: SecretValue::new(key),
        account,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_flag_key_and_derives_account() {
        let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let c = resolve_credentials(Some(key), None).unwrap();
        assert_eq!(c.account, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        assert_eq!(c.private_key.expose(), key);
    }

    #[test]
    fn secret_is_redacted_in_debug_and_display() {
        let s = SecretValue::new("0xdeadbeef".into());
        assert_eq!(format!("{s:?}"), "[REDACTED]");
        assert_eq!(format!("{s}"), "[REDACTED]");
        assert_eq!(s.expose(), "0xdeadbeef");
    }

    #[test]
    fn config_roundtrips_through_toml() {
        let cfg = RisexConfig {
            auth: AuthConfig {
                private_key: Some("0xkey".into()),
                account: Some("0xabc".into()),
            },
            settings: SettingsConfig {
                network: Some("testnet".into()),
                output: Some("table".into()),
                default_market: Some("BTC/USDC".into()),
            },
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: RisexConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.auth.account.as_deref(), Some("0xabc"));
        assert_eq!(back.settings.default_market.as_deref(), Some("BTC/USDC"));
    }

    #[test]
    fn mask_address_keeps_prefix_and_suffix() {
        assert_eq!(mask_address("0x1234567890abcdef"), "0x1234…cdef");
        assert_eq!(mask_address("short"), "****");
    }
}
