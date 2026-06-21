//! One-shot localhost callback server for the browser auth bridge.
//! Binds 127.0.0.1 on a random port, accepts a single signed POST /callback,
//! validates the CSRF `state`, and returns the parsed payload.
use std::io::Read;
use std::time::{Duration, Instant};

use serde_json::Value;
use tiny_http::{Header, Method, Response, Server};

use crate::errors::{Result, RisexError};

pub struct Bridge {
    server: Server,
    port: u16,
    state: String,
}

#[derive(Debug)]
pub struct Callback {
    pub action: String,
    pub account: String,
    pub fields: Value,
}

fn cors_headers() -> Vec<Header> {
    vec![
        Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
        Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"POST, OPTIONS"[..]).unwrap(),
        Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type"[..]).unwrap(),
    ]
}

fn respond(req: tiny_http::Request, status: u16, body: &str) {
    let mut resp = Response::from_string(body).with_status_code(status);
    for h in cors_headers() {
        resp.add_header(h);
    }
    let _ = req.respond(resp);
}

impl Bridge {
    pub fn start() -> Result<Self> {
        let server = Server::http("127.0.0.1:0")
            .map_err(|e| RisexError::Network(format!("failed to start local server: {e}")))?;
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            _ => return Err(RisexError::Network("no TCP port for local server".into())),
        };
        Ok(Self {
            server,
            port,
            state: uuid::Uuid::new_v4().to_string(),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn state(&self) -> &str {
        &self.state
    }

    /// Block until a valid signed callback arrives or the timeout elapses.
    pub fn await_callback(self, timeout: Duration) -> Result<Callback> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RisexError::Network("auth bridge timed out".into()));
            }
            match self.server.recv_timeout(remaining) {
                Ok(Some(mut req)) => {
                    // CORS preflight.
                    if *req.method() == Method::Options {
                        respond(req, 204, "");
                        continue;
                    }
                    // Friendly root page (browser may GET it).
                    if *req.method() == Method::Get {
                        respond(req, 200, "risex auth bridge — you can close this tab.");
                        continue;
                    }
                    if *req.method() != Method::Post || !req.url().starts_with("/callback") {
                        respond(req, 404, "not found");
                        continue;
                    }
                    let mut body = String::new();
                    if req.as_reader().read_to_string(&mut body).is_err() {
                        respond(req, 400, "bad body");
                        continue;
                    }
                    let v: Value = match serde_json::from_str(&body) {
                        Ok(v) => v,
                        Err(_) => {
                            respond(req, 400, "bad json");
                            continue;
                        }
                    };
                    if v.get("state").and_then(|s| s.as_str()) != Some(&self.state) {
                        respond(req, 400, "bad state");
                        continue; // keep waiting; ignore spurious posts
                    }
                    let action = v
                        .get("action")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let account = v
                        .get("account")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    if action.is_empty() || account.is_empty() {
                        respond(req, 400, "missing action/account");
                        continue;
                    }
                    respond(req, 200, "ok");
                    return Ok(Callback {
                        action,
                        account,
                        fields: v,
                    });
                }
                Ok(None) => continue, // timeout tick
                Err(e) => return Err(RisexError::Network(format!("bridge recv error: {e}"))),
            }
        }
    }
}
