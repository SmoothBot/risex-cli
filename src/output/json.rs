use serde_json::Value;

use crate::errors::RisexError;

pub fn render_success(data: &Value) {
    match serde_json::to_string(data) {
        Ok(s) => println!("{s}"),
        Err(_) => println!(r#"{{"error":"parse","message":"JSON serialization failed"}}"#),
    }
}

pub fn render_error(err: &RisexError) {
    let envelope = err.to_json_envelope();
    println!("{}", serde_json::to_string(&envelope).unwrap_or_default());
}
