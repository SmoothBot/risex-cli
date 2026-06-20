//! Client attribution headers. No analytics — agent identification only.
use std::fs;

use crate::config;

pub const CLIENT_NAME: &str = "risex-cli";

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Best-effort detection of the calling agent from environment markers.
pub fn detect_agent() -> &'static str {
    if std::env::var_os("CLAUDECODE").is_some() {
        "claude"
    } else if std::env::var_os("CURSOR_AGENT").is_some() {
        "cursor"
    } else if std::env::var_os("CODEX_SANDBOX").is_some() {
        "codex"
    } else if std::env::var_os("GEMINI_CLI").is_some() {
        "gemini"
    } else if std::env::var_os("VSCODE_PID").is_some() {
        "vscode"
    } else {
        "direct"
    }
}

/// Stable per-install UUID, persisted at `~/.local/share/risex/instance_id`.
/// Falls back to an ephemeral UUID if the data dir is unavailable.
pub fn instance_id() -> String {
    let Ok(dir) = config::data_dir() else {
        return uuid::Uuid::new_v4().to_string();
    };
    let path = dir.join("instance_id");
    if let Ok(existing) = fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(&path, &id);
    id
}

pub fn client_headers() -> Vec<(&'static str, String)> {
    vec![
        ("X-Risex-Client", CLIENT_NAME.to_string()),
        ("X-Risex-Client-Version", version().to_string()),
        ("X-Risex-Agent-Client", detect_agent().to_string()),
        ("X-Risex-Instance-Id", instance_id()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_agent_defaults_to_direct_when_unset() {
        let agent = detect_agent();
        assert!(
            ["direct", "claude", "cursor", "codex", "gemini", "vscode"].contains(&agent),
            "unexpected agent: {agent}"
        );
    }

    #[test]
    fn client_headers_include_required_keys() {
        let headers = client_headers();
        let keys: Vec<&str> = headers.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&"X-Risex-Client"));
        assert!(keys.contains(&"X-Risex-Client-Version"));
        assert!(keys.contains(&"X-Risex-Agent-Client"));
        assert!(keys.contains(&"X-Risex-Instance-Id"));
    }
}
