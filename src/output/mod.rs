//! Output rendering. Data goes to stdout; verbose/warnings to stderr.
mod json;
mod table;

use clap::ValueEnum;
use serde_json::Value;

use crate::errors::RisexError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

/// Structured result of a command: raw JSON plus a table projection.
pub struct CommandOutput {
    pub data: Value,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl CommandOutput {
    pub fn new(data: Value, headers: Vec<String>, rows: Vec<Vec<String>>) -> Self {
        Self { data, headers, rows }
    }

    /// Two-column "Field / Value" projection over `pairs`, with `data` as the
    /// JSON payload.
    pub fn key_value(pairs: Vec<(String, String)>, data: Value) -> Self {
        let rows = pairs.into_iter().map(|(k, v)| vec![k, v]).collect();
        Self {
            data,
            headers: vec!["Field".into(), "Value".into()],
            rows,
        }
    }

    /// Single-cell message output.
    pub fn message(msg: &str) -> Self {
        Self {
            data: serde_json::json!({ "message": msg }),
            headers: vec!["Message".into()],
            rows: vec![vec![msg.to_string()]],
        }
    }
}

pub fn render(format: OutputFormat, output: &CommandOutput) {
    match format {
        OutputFormat::Table => table::render(output),
        OutputFormat::Json => json::render_success(&output.data),
    }
}

pub fn render_error(format: OutputFormat, err: &RisexError) {
    match format {
        OutputFormat::Table => eprintln!("Error [{}]: {}", err.category(), err),
        OutputFormat::Json => json::render_error(err),
    }
}

pub fn verbose(msg: &str) {
    eprintln!("[verbose] {msg}");
}

pub fn warn(msg: &str) {
    eprintln!("Warning: {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_value_builds_two_column_rows() {
        let out = CommandOutput::key_value(
            vec![("network".into(), "testnet".into())],
            json!({"network": "testnet"}),
        );
        assert_eq!(out.headers, vec!["Field".to_string(), "Value".to_string()]);
        assert_eq!(out.rows, vec![vec!["network".to_string(), "testnet".to_string()]]);
    }

    #[test]
    fn message_output_carries_text() {
        let out = CommandOutput::message("done");
        assert_eq!(out.rows, vec![vec!["done".to_string()]]);
        assert_eq!(out.data["message"], "done");
    }
}
