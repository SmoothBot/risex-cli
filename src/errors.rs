//! Unified error type for the RISEx CLI. Categories are stable strings so
//! callers (and agents driving the CLI) can branch on them.
use serde_json::json;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, RisexError>;

#[derive(Debug, Error)]
pub enum RisexError {
    #[error("{0}")]
    Api(String),
    #[error("{0}")]
    Auth(String),
    #[error("{message}")]
    RateLimit {
        message: String,
        retryable: bool,
        suggestion: Option<String>,
        docs_url: Option<String>,
    },
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Signing(String),
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    WebSocket(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Parse(String),
}

impl RisexError {
    pub fn category(&self) -> &'static str {
        match self {
            Self::Api(_) => "api",
            Self::Auth(_) => "auth",
            Self::RateLimit { .. } => "rate_limit",
            Self::Validation(_) => "validation",
            Self::Network(_) => "network",
            Self::Signing(_) => "signing",
            Self::Config(_) => "config",
            Self::WebSocket(_) => "websocket",
            Self::Io(_) => "io",
            Self::Parse(_) => "parse",
        }
    }

    pub fn to_json_envelope(&self) -> serde_json::Value {
        match self {
            Self::RateLimit {
                message,
                retryable,
                suggestion,
                docs_url,
            } => {
                let mut env = json!({
                    "error": "rate_limit",
                    "message": message,
                    "retryable": retryable,
                });
                if let Some(s) = suggestion {
                    env["suggestion"] = json!(s);
                }
                if let Some(d) = docs_url {
                    env["docs_url"] = json!(d);
                }
                env
            }
            other => json!({
                "error": other.category(),
                "message": other.to_string(),
            }),
        }
    }
}

impl From<reqwest::Error> for RisexError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() || e.is_connect() {
            RisexError::Network(e.to_string())
        } else {
            RisexError::Parse(e.to_string())
        }
    }
}

impl From<serde_json::Error> for RisexError {
    fn from(e: serde_json::Error) -> Self {
        RisexError::Parse(e.to_string())
    }
}

impl From<toml::de::Error> for RisexError {
    fn from(e: toml::de::Error) -> Self {
        RisexError::Config(format!("TOML parse error: {e}"))
    }
}

impl From<toml::ser::Error> for RisexError {
    fn from(e: toml::ser::Error) -> Self {
        RisexError::Config(format!("TOML serialize error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_strings_are_stable() {
        assert_eq!(RisexError::Api("x".into()).category(), "api");
        assert_eq!(RisexError::Validation("x".into()).category(), "validation");
        assert_eq!(
            RisexError::RateLimit {
                message: "x".into(),
                retryable: true,
                suggestion: None,
                docs_url: None
            }
            .category(),
            "rate_limit"
        );
    }

    #[test]
    fn envelope_has_error_and_message_fields() {
        let env = RisexError::Auth("bad token".into()).to_json_envelope();
        assert_eq!(env["error"], "auth");
        assert_eq!(env["message"], "bad token");
    }

    #[test]
    fn rate_limit_envelope_includes_retryable() {
        let env = RisexError::RateLimit {
            message: "slow down".into(),
            retryable: true,
            suggestion: Some("wait a bit".into()),
            docs_url: None,
        }
        .to_json_envelope();
        assert_eq!(env["error"], "rate_limit");
        assert_eq!(env["retryable"], true);
        assert_eq!(env["suggestion"], "wait a bit");
    }
}
