//! mcp/introspect.rs — Minimal JSON-RPC 2.0 stdio client for MCP.
//!
//! Port of CCO's `src/mcp-introspector.mjs`. Used by the security
//! scanner to enumerate a server's tool definitions so we can:
//!   - hash each tool's canonical form into the baseline (Layer 3)
//!   - run Layer 1 + Layer 2 over descriptions and param names
//!
//! **Trust model: this spawns the MCP server's actual process. The
//! server binary is untrusted code we are deliberately executing
//! (same trust model as Claude Code itself). Introspection is only
//! invoked as part of an explicit user-triggered scan — never on
//! app startup or in the background.**
//!
//! Hand-rolled rather than `rmcp`/`mcp-rust-sdk` to stay thin: we only
//! need `initialize` + `tools/list`. HTTP/SSE is out of scope (the CCO
//! server list is stdio-only in the user flow).

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::WardError;

/// One tool returned by `tools/list`. `hash` is the SHA-256 hex digest
/// of `canonical_json({ name, description, inputSchema })` — the same
/// canonical form CCO uses, so baselines are interchangeable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub input_schema: serde_json::Value,
    pub hash: String,
}

/// Spawn the given MCP server, send `initialize` + `tools/list`, and
/// parse the response. Returns the tool list with per-tool hashes.
///
/// Timeouts (per the plan):
///   - 5 s to spawn the process
///   - 10 s after spawn for `tools/list` to respond
///
/// Any failure (spawn / pipe / parse / timeout) returns
/// [`WardError::McpIntrospectFailed`] with a one-line message.
pub async fn introspect_server(
    command: &str,
    args: &[String],
) -> Result<Vec<ToolDef>, WardError> {
    let cmd = command.to_string();
    let args_owned: Vec<String> = args.to_vec();
    // Heavy I/O — move off the async runtime.
    tauri::async_runtime::spawn_blocking(move || introspect_blocking(&cmd, &args_owned))
        .await
        .map_err(|e| WardError::McpIntrospectFailed(format!("join error: {e}")))?
}

fn introspect_blocking(command: &str, args: &[String]) -> Result<Vec<ToolDef>, WardError> {
    // ── Spawn ────────────────────────────────────────────────────────
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // mirror CCO — many servers log here
        .spawn()
        .map_err(|e| WardError::McpIntrospectFailed(format!("spawn failed: {e}")))?;

    let mut stdin = child.stdin.take().ok_or_else(|| {
        WardError::McpIntrospectFailed("no stdin on child process".into())
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        WardError::McpIntrospectFailed("no stdout on child process".into())
    })?;

    // A dedicated reader thread streams newline-delimited JSON-RPC
    // messages back to us via a channel. That keeps the read path off
    // the blocking-call path inside our state machine.
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    if tx.send(line.clone()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // ── JSON-RPC handshake ───────────────────────────────────────────
    let init_id = next_id();
    let init_req = jsonrpc_request(
        init_id,
        "initialize",
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "ward-security-scanner", "version": "0.1.0" }
        }),
    );
    write_message(&mut stdin, &init_req)?;

    // Drain lines until we see the initialize response (the server may
    // print log lines before it — CCO skips non-JSON lines too).
    let _init_resp = wait_for_response(&rx, init_id, Duration::from_secs(10))?;

    // `initialized` notification (no response expected, no id)
    let initialized = jsonrpc_notification("notifications/initialized", serde_json::json!({}));
    write_message(&mut stdin, &initialized)?;

    // ── tools/list ───────────────────────────────────────────────────
    let list_id = next_id();
    let list_req = jsonrpc_request(list_id, "tools/list", serde_json::json!({}));
    write_message(&mut stdin, &list_req)?;

    let tools_resp = wait_for_response(&rx, list_id, Duration::from_secs(10))?;

    // ── Parse + hash ─────────────────────────────────────────────────
    let tools_arr = tools_resp
        .get("tools")
        .and_then(|v| v.as_array())
        .ok_or_else(|| WardError::McpIntrospectFailed("no tools[] in response".into()))?;

    let mut out = Vec::with_capacity(tools_arr.len());
    for raw in tools_arr {
        let name = raw
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let description = raw
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let input_schema = raw
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let hash = hash_tool(&name, &description, &input_schema);
        out.push(ToolDef {
            name,
            description,
            input_schema,
            hash,
        });
    }

    // ── Reap ─────────────────────────────────────────────────────────
    let _ = child.kill();
    let _ = child.wait();

    Ok(out)
}

/// Pure helper exposed for tests + rule-engine callers.
pub fn hash_tool(name: &str, description: &str, input_schema: &serde_json::Value) -> String {
    // CCO serializes `{ name, description, inputSchema }` then hashes
    // the UTF-8 bytes. We mirror that verbatim.
    let payload = serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    });
    let bytes = serde_json::to_vec(&payload).expect("json vec");
    let mut h = Sha256::new();
    h.update(&bytes);
    hex_lower(&h.finalize())
}

// ── JSON-RPC plumbing ───────────────────────────────────────────────

static RPC_ID: AtomicU64 = AtomicU64::new(0);

fn next_id() -> u64 { RPC_ID.fetch_add(1, Ordering::Relaxed) + 1 }

fn jsonrpc_request(id: u64, method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

fn jsonrpc_notification(method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

fn write_message<W: Write>(w: &mut W, msg: &serde_json::Value) -> Result<(), WardError> {
    let line = serde_json::to_string(msg)
        .map_err(|e| WardError::McpIntrospectFailed(format!("encode: {e}")))?;
    writeln!(w, "{line}")
        .map_err(|e| WardError::McpIntrospectFailed(format!("write: {e}")))?;
    w.flush()
        .map_err(|e| WardError::McpIntrospectFailed(format!("flush: {e}")))?;
    Ok(())
}

/// Wait for the response whose `id == expected_id`. Skips non-JSON log
/// lines and notifications. Returns the `result` object.
fn wait_for_response(
    rx: &mpsc::Receiver<String>,
    expected_id: u64,
    timeout: Duration,
) -> Result<serde_json::Value, WardError> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(WardError::McpIntrospectFailed(format!(
                "timeout waiting for response id={expected_id}"
            )));
        }
        match rx.recv_timeout(remaining) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }
                let msg: serde_json::Value = match serde_json::from_str(trimmed) {
                    Ok(v) => v,
                    Err(_) => continue, // skip log lines
                };
                if msg.get("id").and_then(|v| v.as_u64()) != Some(expected_id) {
                    continue;
                }
                if let Some(err) = msg.get("error") {
                    return Err(WardError::McpIntrospectFailed(format!("RPC error: {err}")));
                }
                return msg
                    .get("result")
                    .cloned()
                    .ok_or_else(|| {
                        WardError::McpIntrospectFailed("response missing result".into())
                    });
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(WardError::McpIntrospectFailed(format!(
                    "timeout waiting for response id={expected_id}"
                )));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(WardError::McpIntrospectFailed(
                    "server closed stdout before responding".into(),
                ));
            }
        }
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Hash is deterministic for the same canonical input.
    #[test]
    fn hash_tool_is_deterministic() {
        let a = hash_tool(
            "echo",
            "Echoes input back",
            &serde_json::json!({"type": "object"}),
        );
        let b = hash_tool(
            "echo",
            "Echoes input back",
            &serde_json::json!({"type": "object"}),
        );
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    /// Hash changes when any field changes (name, description, schema).
    #[test]
    fn hash_tool_changes_on_any_field() {
        let base = hash_tool("echo", "Echoes back", &serde_json::json!({}));
        assert_ne!(base, hash_tool("echo2", "Echoes back", &serde_json::json!({})));
        assert_ne!(base, hash_tool("echo", "Different", &serde_json::json!({})));
        assert_ne!(
            base,
            hash_tool("echo", "Echoes back", &serde_json::json!({"type": "object"}))
        );
    }

    /// Hash is canonical — serde_json's default `Map` is a BTreeMap,
    /// so different insertion order produces the same hash. That
    /// matches `JSON.stringify` for most realistic MCP tool schemas.
    #[test]
    fn hash_tool_is_canonical_regardless_of_key_order() {
        let a = hash_tool("t", "d", &serde_json::json!({"a": 1, "b": 2}));
        let b = hash_tool("t", "d", &serde_json::json!({"b": 2, "a": 1}));
        assert_eq!(a, b);
    }

    /// End-to-end against a fake MCP server. The fake server is a
    /// shell script that replies with a valid initialize response and
    /// a tools/list response. We verify Ward parses them, hashes them,
    /// and returns the right tool count + hashable names.
    #[test]
    fn introspect_against_fake_server() {
        // Minimal MCP server: respond to initialize, then to tools/list.
        // Uses `printf` for unbuffered output. Sleeps 50ms between writes
        // so the child's stdout is interleaved line-by-line.
        let script = r#"#!/bin/sh
# Read stdin until we see two newline-delimited JSON-RPC requests.
# Always respond with the right shape.
read -r init_line
echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"fake","version":"0.0.1"}}}'
sleep 0.05
read -r list_line
echo '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"hello","description":"Greets the user","inputSchema":{"type":"object","properties":{"name":{"type":"string"}}}},{"name":"echo","description":"Repeats input","inputSchema":{"type":"object"}}]}}'
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake-mcp.sh");
        std::fs::write(&path, script).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        std::fs::set_permissions(&path, {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
            perms
        })
        .unwrap();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let res = rt.block_on(introspect_server(
            path.to_str().unwrap(),
            &[],
        ));
        let tools = res.expect("introspect should succeed");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "hello");
        assert_eq!(tools[1].name, "echo");
        assert!(tools.iter().all(|t| !t.hash.is_empty() && t.hash.len() == 64));
        // Same input → same hash.
        let again = hash_tool(&tools[0].name, &tools[0].description, &tools[0].input_schema);
        assert_eq!(tools[0].hash, again);
    }

    /// Spawning a nonexistent binary returns `McpIntrospectFailed`.
    #[test]
    fn introspect_missing_binary_fails() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let res = rt.block_on(introspect_server(
            "/nonexistent/binary/ward-test-mcp",
            &[],
        ));
        match res {
            Err(WardError::McpIntrospectFailed(_)) => {}
            other => panic!("expected McpIntrospectFailed, got {other:?}"),
        }
    }
}