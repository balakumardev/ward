//! mcp/server.rs — Ward headless MCP server (stdio JSON-RPC 2.0).
//!
//! Plan 11 exposes Ward's existing scan/move/delete/destinations/security
//! operations as the 5 MCP tools that mirror CCO's `mcp-server.mjs`
//! verbatim (`scan_inventory`, `move_item`, `delete_item`,
//! `list_destinations`, `audit_security`). Each tool delegates to the
//! core impl — no logic is duplicated here.
//!
//! Transport: stdio with LSP-style `Content-Length` framing, matching the
//! MCP spec ("JSON-RPC over stdio" with Content-Length headers). The
//! hand-rolled design follows `mcp/introspect.rs`'s "keep it small"
//! ethos — we don't pull in `rmcp`/`mcp-rust-sdk` because we only need
//! `initialize` + `tools/list` + `tools/call`.
//!
//! Headless: no Tauri window. `main.rs --mcp` opens the loop here and
//! runs until EOF.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::commands;
use crate::error::WardError;
use crate::harness::adapters::claude_ops::{ClaudeOps, get_valid_destinations};
use crate::harness::{Ctx, HarnessOps, Registry};
use crate::model::{HarnessItem, ScanResult};
use crate::security::scan::{self, ScanOptions};

// ── JSON-RPC envelope ──────────────────────────────────────────────────

/// One inbound JSON-RPC 2.0 envelope. Per spec, `id` is required for
/// requests (the wire shape is a number/string/null) and absent for
/// notifications; we accept `null` plus any JSON value here and we
/// echo it back unchanged in the response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// One outbound JSON-RPC envelope. Either `result` or `error` is set,
/// never both. `id` always echoes the request's id (or null for parse
/// failures with no recoverable id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    pub fn result(id: Value, result: Value) -> Self {
        JsonRpcResponse { jsonrpc: "2.0", id, result: Some(result), error: None }
    }
    pub fn error(id: Value, code: i32, message: impl Into<String>) -> Self {
        JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError { code, message: message.into(), data: None }),
        }
    }
}

// Standard JSON-RPC error codes (kept inline so we don't need a new dep).
// const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
// const INVALID_PARAMS_I32: i32 = -32602; // typo suppression
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;

// ── Framing: Content-Length read/write ──────────────────────────────────

/// Read one LSP-framed JSON-RPC message from `reader`. Returns:
///   - `Ok(Some(req))` for a complete message
///   - `Ok(None)` on clean EOF (no further messages)
///   - `Err(_)` on parse / IO failure
///
/// Framing rules (matches the MCP spec + CCO's transport):
///   - Headers are CRLF (`\r\n` or `\n`) terminated, ASCII.
///   - `Content-Length: <bytes>` is the only required header.
///   - Headers terminate on a blank line.
///   - The body is exactly `Content-Length` raw bytes — no trailing CRLF.
pub fn read_message<R: BufRead>(reader: &mut R) -> Result<Option<JsonRpcRequest>, WardError> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).map_err(|e| {
            WardError::McpIntrospectFailed(format!("read header: {e}"))
        })?;
        if n == 0 {
            // EOF before any header — clean shutdown.
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(|c: char| c == '\r' || c == '\n');
        if trimmed.is_empty() {
            // End of headers — body (if any) follows.
            break;
        }
        // Case-insensitive header match (LSP/MCP allow fold-style).
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            let v = rest.trim().parse::<usize>().map_err(|e| {
                WardError::McpIntrospectFailed(format!("bad Content-Length: {e}"))
            })?;
            content_length = Some(v);
        }
        // Unknown headers are ignored (Content-Type, etc. — MCP doesn't
        // require them, but a chatty client might send them).
    }
    let len = content_length.ok_or_else(|| {
        WardError::McpIntrospectFailed("missing Content-Length header".into())
    })?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).map_err(|e| {
        WardError::McpIntrospectFailed(format!("read body: {e}"))
    })?;
    let req: JsonRpcRequest = serde_json::from_slice(&buf).map_err(|e| {
        WardError::McpIntrospectFailed(format!("parse JSON: {e}"))
    })?;
    Ok(Some(req))
}

/// Write a single LSP-framed JSON-RPC message to `writer`. Always
/// flushes so the client sees the reply promptly.
pub fn write_message<W: Write>(writer: &mut W, response: &JsonRpcResponse) -> Result<(), WardError> {
    let body = serde_json::to_vec(response).map_err(|e| {
        WardError::McpIntrospectFailed(format!("encode response: {e}"))
    })?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len()).map_err(|e| {
        WardError::McpIntrospectFailed(format!("write header: {e}"))
    })?;
    writer.write_all(&body).map_err(|e| {
        WardError::McpIntrospectFailed(format!("write body: {e}"))
    })?;
    writer.flush().map_err(|e| {
        WardError::McpIntrospectFailed(format!("flush: {e}"))
    })?;
    Ok(())
}

// ── Tool catalog (CCO parity) ──────────────────────────────────────────

/// Actionable category enum — used as the `category` input on every
/// mutation/destinations tool. Matches CCO's
/// `actionCategories.filter(c.movable || c.deletable)`. The ward-side
/// truth lives in `claude_ops::ClaudeOps::move_item` and
/// `delete_item`; this list is the public input schema for the MCP
/// tools. Any value outside this set is rejected at the JSON-RPC layer.
const ACTION_CATEGORIES: &[&str] = &[
    "memory", "skill", "mcp", "command", "agent", "plan", "rule", "session",
];

fn category_schema() -> Value {
    json!({
        "type": "string",
        "enum": ACTION_CATEGORIES,
        "description": "Category of item. Must be one of: memory, skill, mcp, command, agent, plan, rule, session.",
    })
}

/// The 5 tool definitions exposed by `tools/list`. Names + input schemas
/// mirror CCO's `mcp-server.mjs` verbatim so existing muscle memory
/// carries over. Returned with the right shape for the MCP protocol.
pub fn tool_definitions() -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(5);

    // 1. scan_inventory — no input args.
    out.push(json!({
        "name": "scan_inventory",
        "description": "Scan all Claude Code configurations across global and project scopes. Returns skills, memories, MCP servers, commands, agents, plans, rules, configs, hooks, plugins, sessions, and settings with file paths and metadata.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    }));

    // 2. move_item — category, name, fromScopeId, toScopeId.
    out.push(json!({
        "name": "move_item",
        "description": "Move a Claude Code configuration item from one scope to another. Run scan_inventory first to see available items and scope IDs.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "category": category_schema(),
                "name": {
                    "type": "string",
                    "description": "Name of the item (as shown in scan_inventory results).",
                },
                "fromScopeId": {
                    "type": "string",
                    "description": "Source scope ID (e.g. \"global\" or the encoded project directory name).",
                },
                "toScopeId": {
                    "type": "string",
                    "description": "Destination scope ID.",
                },
            },
            "required": ["category", "name", "fromScopeId", "toScopeId"],
        },
    }));

    // 3. delete_item — category, name, scopeId.
    out.push(json!({
        "name": "delete_item",
        "description": "Delete a Claude Code configuration item. Run scan_inventory first to see available items and scope IDs.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "category": category_schema(),
                "name": {
                    "type": "string",
                    "description": "Name of the item (as shown in scan_inventory results).",
                },
                "scopeId": {
                    "type": "string",
                    "description": "Scope ID where the item lives.",
                },
            },
            "required": ["category", "name", "scopeId"],
        },
    }));

    // 4. list_destinations — category, name, scopeId.
    out.push(json!({
        "name": "list_destinations",
        "description": "List valid destination scopes for a specific item. Shows where this item can be moved to.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "category": category_schema(),
                "name": {
                    "type": "string",
                    "description": "Name of the item.",
                },
                "scopeId": {
                    "type": "string",
                    "description": "Current scope ID of the item.",
                },
            },
            "required": ["category", "name", "scopeId"],
        },
    }));

    // 5. audit_security — no input args.
    out.push(json!({
        "name": "audit_security",
        "description": "Scan all MCP servers for security vulnerabilities. Runs pattern-based detection over each server's config (command, args, description). Returns findings with severity levels and baseline comparison.",
        "inputSchema": {
            "type": "object",
            "properties": {},
        },
    }));

    out
}

// ── Handler (sync, dispatchable from a test) ───────────────────────────

/// One handler instance owns a scanned home + scope list + scan cache.
/// Constructed once at server start; lives until EOF.
pub struct Handler {
    pub home: PathBuf,
    /// Leaked `'static` so `Ctx<'static>` matches the existing signature
    /// on `HarnessOps` (which expects `Ctx<'a, 'static>` semantically).
    /// This is exactly what `commands::harness_ctx` already does — same
    /// one-leak-per-process trade-off, acceptable for a long-lived server.
    ctx: Ctx<'static>,
    registry: Registry,
    /// Scope list snapshot, refreshed lazily on cache miss.
    scopes: Vec<crate::model::Scope>,
    /// Last successful `scan()`. Used by move/delete/list_destinations
    /// to look up the on-disk `HarnessItem` for a `category + name + scopeId`.
    cache: Mutex<Option<ScanResult>>,
}

impl Handler {
    /// Build the handler against `home`. Initializes the registry and
    /// populates the initial scope list + scan cache. Returns the error
    /// from the harness if scope discovery fails.
    pub fn new(home: PathBuf) -> Result<Self, WardError> {
        let home_static: &'static Path = Box::leak(home.clone().into_boxed_path());
        let ctx = Ctx { home: home_static, cwd: None };
        let registry = commands::build_registry();
        let adapter = registry.get("claude").ok_or_else(|| {
            WardError::HarnessUnavailable("claude".into())
        })?;
        let scopes = adapter.discover_scopes(&ctx)?;
        let initial_scan = commands::scan_impl(&registry, &home, "claude").ok();
        Ok(Self {
            home,
            ctx,
            registry,
            scopes,
            cache: Mutex::new(initial_scan),
        })
    }

    /// Test-only constructor that accepts a pre-built state. Skips
    /// the leaked-home trick so unit tests don't permanently allocate.
    #[cfg(test)]
    pub fn new_for_test(home: PathBuf, registry: Registry, scopes: Vec<crate::model::Scope>) -> Self {
        let home_static: &'static Path = Box::leak(home.clone().into_boxed_path());
        let ctx = Ctx { home: home_static, cwd: None };
        Self { home, ctx, registry, scopes, cache: Mutex::new(None) }
    }

    /// True if `category` is in the actionable enum.
    pub fn is_known_category(category: &str) -> bool {
        ACTION_CATEGORIES.contains(&category)
    }

    /// Top-level dispatch — picks the right handler by `method`.
    pub fn handle(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        match req.method.as_str() {
            "initialize" => self.handle_initialize(req),
            "tools/list" => self.handle_tools_list(req),
            "tools/call" => self.handle_tools_call(req),
            // MCP notifications are fire-and-forget; we ack with a
            // null-id result if id is provided, otherwise ignore.
            "notifications/initialized" | "notifications/cancelled" => {
                JsonRpcResponse::result(req.id, json!({}))
            }
            _ => JsonRpcResponse::error(
                req.id,
                METHOD_NOT_FOUND,
                format!("Method not found: {}", req.method),
            ),
        }
    }

    fn handle_initialize(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        JsonRpcResponse::result(
            req.id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "ward",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        )
    }

    fn handle_tools_list(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        JsonRpcResponse::result(
            req.id,
            json!({ "tools": tool_definitions() }),
        )
    }

    fn handle_tools_call(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let name = req
            .params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let args = req
            .params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let call_id = req.id.clone();

        match name.as_str() {
            "scan_inventory" => self.tool_scan_inventory(call_id),
            "move_item" => self.tool_move_item(call_id, args),
            "delete_item" => self.tool_delete_item(call_id, args),
            "list_destinations" => self.tool_list_destinations(call_id, args),
            "audit_security" => self.tool_audit_security(call_id),
            other => JsonRpcResponse::error(
                call_id,
                METHOD_NOT_FOUND,
                format!("Unknown tool: {other}"),
            ),
        }
    }

    // ── Tool impls (each returns a `result` value or a JSON-RPC error)
    //
    // Tool-call errors (item not found, validation failure, IO error)
    // are surfaced as `isError: true` in the result with a JSON payload
    // describing the failure — same convention as CCO. Protocol-level
    // errors (bad tool name, bad schema) bubble up as JSON-RPC errors.

    fn tool_scan_inventory(&self, id: Value) -> JsonRpcResponse {
        match commands::scan_impl(&self.registry, &self.home, "claude") {
            Ok(result) => {
                self.store_cache(result.clone());
                let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                JsonRpcResponse::result(id, content_text(&text, false))
            }
            Err(e) => tool_error(id, &e),
        }
    }

    fn tool_move_item(&self, id: Value, args: Value) -> JsonRpcResponse {
        let parsed = match parse_move_args(&args) {
            Ok(p) => p,
            Err(m) => return JsonRpcResponse::error(id, INVALID_PARAMS, m),
        };
        if !Self::is_known_category(&parsed.category) {
            return JsonRpcResponse::error(
                id,
                INVALID_PARAMS,
                format!("Unknown category: {}", parsed.category),
            );
        }
        let Some(item) = self.find_item(&parsed.category, &parsed.name, &parsed.from_scope_id) else {
            return JsonRpcResponse::result(
                id,
                content_text(&not_found_payload(&parsed.category, &parsed.name, &parsed.from_scope_id), false),
            );
        };
        let ops: &dyn HarnessOps = &ClaudeOps;
        match ops.move_item(&self.ctx, &item, &parsed.to_scope_id, &self.scopes) {
            Ok(info) => {
                self.refresh_cache();
                let payload = serde_json::json!({ "ok": true, "restoreInfo": info });
                JsonRpcResponse::result(id, content_text(&payload.to_string(), false))
            }
            Err(e) => tool_error(id, &e),
        }
    }

    fn tool_delete_item(&self, id: Value, args: Value) -> JsonRpcResponse {
        let parsed = match parse_delete_args(&args) {
            Ok(p) => p,
            Err(m) => return JsonRpcResponse::error(id, INVALID_PARAMS, m),
        };
        if !Self::is_known_category(&parsed.category) {
            return JsonRpcResponse::error(
                id,
                INVALID_PARAMS,
                format!("Unknown category: {}", parsed.category),
            );
        }
        let Some(item) = self.find_item(&parsed.category, &parsed.name, &parsed.scope_id) else {
            return JsonRpcResponse::result(
                id,
                content_text(&not_found_payload(&parsed.category, &parsed.name, &parsed.scope_id), false),
            );
        };
        let ops: &dyn HarnessOps = &ClaudeOps;
        match ops.delete_item(&self.ctx, &item, &self.scopes) {
            Ok(info) => {
                self.refresh_cache();
                let payload = serde_json::json!({ "ok": true, "restoreInfo": info });
                JsonRpcResponse::result(id, content_text(&payload.to_string(), false))
            }
            Err(e) => tool_error(id, &e),
        }
    }

    fn tool_list_destinations(&self, id: Value, args: Value) -> JsonRpcResponse {
        let parsed = match parse_dest_args(&args) {
            Ok(p) => p,
            Err(m) => return JsonRpcResponse::error(id, INVALID_PARAMS, m),
        };
        if !Self::is_known_category(&parsed.category) {
            return JsonRpcResponse::error(
                id,
                INVALID_PARAMS,
                format!("Unknown category: {}", parsed.category),
            );
        }
        let Some(item) = self.find_item(&parsed.category, &parsed.name, &parsed.scope_id) else {
            return JsonRpcResponse::result(
                id,
                content_text(&not_found_payload(&parsed.category, &parsed.name, &parsed.scope_id), false),
            );
        };
        let dests = get_valid_destinations(&self.home, &item, &self.scopes);
        let payload = serde_json::json!({
            "ok": true,
            "destinations": dests,
            "currentScopeId": item.scope_id,
        });
        let text = serde_json::to_string_pretty(&payload).unwrap_or_default();
        JsonRpcResponse::result(id, content_text(&text, false))
    }

    fn tool_audit_security(&self, id: Value) -> JsonRpcResponse {
        let result = match commands::scan_impl(&self.registry, &self.home, "claude") {
            Ok(r) => r,
            Err(e) => return tool_error(id, &e),
        };
        // Refresh cache; the items list is what audit_security operates on.
        self.store_cache(result.clone());
        let opts = ScanOptions { run_judge: false };
        match scan::scan(&result.items, &opts) {
            Ok(sr) => {
                let text = serde_json::to_string_pretty(&sr).unwrap_or_default();
                JsonRpcResponse::result(id, content_text(&text, false))
            }
            Err(e) => tool_error(id, &e),
        }
    }

    // ── Cache helpers ──

    fn store_cache(&self, result: ScanResult) {
        if let Ok(mut g) = self.cache.lock() {
            *g = Some(result);
        }
    }

    fn refresh_cache(&self) {
        if let Ok(fresh) = commands::scan_impl(&self.registry, &self.home, "claude") {
            self.store_cache(fresh);
        }
    }

    /// Find an item in the latest cached scan, matching CCO's
    /// `(category, name|fileName, scopeId)` lookup. The cache reflects
    /// the most recent scan (initial + post-mutation refresh).
    fn find_item(&self, category: &str, name: &str, scope_id: &str) -> Option<HarnessItem> {
        let cache = self.cache.lock().ok()?;
        let scan = cache.as_ref()?;
        scan.items
            .iter()
            .find(|i| {
                i.category == category
                    && (i.name == name || i.path.ends_with(name) || i.path == name)
                    && i.scope_id == scope_id
            })
            .cloned()
    }
}

// ── Arg parsing ────────────────────────────────────────────────────────

struct MoveArgs {
    category: String,
    name: String,
    from_scope_id: String,
    to_scope_id: String,
}

struct ItemArgs {
    category: String,
    name: String,
    scope_id: String,
}

fn parse_move_args(args: &Value) -> Result<MoveArgs, String> {
    Ok(MoveArgs {
        category: need_string(args, "category")?,
        name: need_string(args, "name")?,
        from_scope_id: need_string(args, "fromScopeId")?,
        to_scope_id: need_string(args, "toScopeId")?,
    })
}

fn parse_delete_args(args: &Value) -> Result<ItemArgs, String> {
    Ok(ItemArgs {
        category: need_string(args, "category")?,
        name: need_string(args, "name")?,
        scope_id: need_string(args, "scopeId")?,
    })
}

fn parse_dest_args(args: &Value) -> Result<ItemArgs, String> {
    Ok(ItemArgs {
        category: need_string(args, "category")?,
        name: need_string(args, "name")?,
        scope_id: need_string(args, "scopeId")?,
    })
}

fn need_string(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing required string argument: {key}"))
}

// ── Result builders ────────────────────────────────────────────────────

fn content_text(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

fn not_found_payload(category: &str, name: &str, scope_id: &str) -> String {
    let payload = serde_json::json!({
        "ok": false,
        "error": format!(
            "Item not found: {category} \"{name}\" in scope \"{scope_id}\". \
             Run scan_inventory first to see available items."
        ),
    });
    payload.to_string()
}

fn tool_error(id: Value, e: &WardError) -> JsonRpcResponse {
    let payload = serde_json::json!({ "ok": false, "error": e.to_string() });
    JsonRpcResponse::result(id, content_text(&payload.to_string(), true))
}

// Suppress unused-import warning for the variant constant from the
// standard codes — kept inline so future readers see them.
#[allow(dead_code)]
const INVALID_REQUEST_CODE: i32 = INVALID_REQUEST;
#[allow(dead_code)]
const INTERNAL_ERROR_CODE: i32 = INTERNAL_ERROR;

// ── Stdio entry point (sync — no async needed for blocking IO) ──────────

/// Run the MCP server on stdio until EOF (or an unrecoverable IO
/// error). Writes all logging to stderr; stdout is exclusively
/// reserved for JSON-RPC frames so MCP clients see a clean stream.
///
/// This is the function `main.rs --mcp` invokes. It constructs a
/// `Handler` against the user's `$HOME`, then loops reading framed
/// messages and dispatching them. The handler caches scope/scan state
/// across the session so each request is cheap.
pub fn run_stdio_server() -> Result<(), WardError> {
    let home = dirs::home_dir().ok_or_else(|| {
        WardError::NotFound("HOME directory".into())
    })?;
    let handler = Handler::new(home)?;
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    loop {
        let msg = match read_message(&mut reader) {
            Ok(Some(m)) => m,
            Ok(None) => return Ok(()), // clean EOF
            Err(e) => {
                eprintln!("ward-mcp: read error: {e}");
                return Err(e);
            }
        };
        let response = handler.handle(msg);
        if let Err(e) = write_message(&mut writer, &response) {
            eprintln!("ward-mcp: write error: {e}");
            return Err(e);
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Round-trip: write a message to a cursor, read it back, and
    /// verify the parsed shape is identical. This is the load-bearing
    /// test that pins our framing to CCO's.
    #[test]
    fn framing_round_trip_simple_message() {
        // We serialize a JsonRpcRequest directly so the on-disk shape
        // includes `method`. (write_message uses the standard envelope,
        // so we keep it generic — we hand a serde_json::Value-shaped
        // request here for clarity.)
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(1),
            method: "ping".into(),
            params: json!({}),
        };
        let body = serde_json::to_vec(&req).unwrap();
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        buf.extend_from_slice(&body);

        // Confirm the header is exactly "Content-Length: N\r\n\r\n"
        let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let header = std::str::from_utf8(&buf[..header_end]).unwrap();
        assert!(header.starts_with("Content-Length: "));
        let expected_len = buf.len() - header_end - 4;
        assert_eq!(
            header,
            format!("Content-Length: {expected_len}").as_str(),
            "header must encode the body length"
        );

        // Read back.
        let mut cur = Cursor::new(buf);
        let parsed = read_message(&mut cur).unwrap().expect("got a message");
        assert_eq!(parsed.method, "ping");
        assert_eq!(parsed.jsonrpc, "2.0");
    }

    /// Multiple messages on the same stream are read back in order.
    /// This is what an MCP client (sending several tools/list + call
    /// pairs) will look like.
    #[test]
    fn framing_round_trip_multiple_messages() {
        let mut buf: Vec<u8> = Vec::new();
        for i in 1..=3 {
            let req = JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: json!(i),
                method: "tools/call".into(),
                params: json!({ "name": format!("t{i}"), "arguments": {} }),
            };
            let body = serde_json::to_vec(&req).unwrap();
            buf.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
            buf.extend_from_slice(&body);
        }
        let mut cur = Cursor::new(buf);
        for i in 1..=3 {
            let m = read_message(&mut cur).unwrap().unwrap();
            assert_eq!(m.id, json!(i));
            assert_eq!(m.params["name"].as_str().unwrap(), format!("t{i}"));
        }
        // EOF after the last message.
        assert!(read_message(&mut cur).unwrap().is_none());
    }

    /// EOF before any header returns `Ok(None)` so the loop can exit
    /// cleanly.
    #[test]
    fn framing_eof_before_headers_returns_none() {
        let buf: Vec<u8> = Vec::new();
        let mut cur = Cursor::new(buf);
        assert!(read_message(&mut cur).unwrap().is_none());
    }

    /// Missing Content-Length header is a hard error (matches MCP).
    #[test]
    fn framing_missing_content_length_errors() {
        let buf: Vec<u8> = b"\r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\"}\n".to_vec();
        let mut cur = Cursor::new(buf);
        match read_message(&mut cur) {
            Err(WardError::McpIntrospectFailed(_)) => {}
            other => panic!("expected McpIntrospectFailed, got {other:?}"),
        }
    }

    /// Truncated body (Content-Length says 100 but only 50 bytes are
    /// buffered) is a hard error.
    #[test]
    fn framing_truncated_body_errors() {
        let buf: Vec<u8> = b"Content-Length: 100\r\n\r\n{\"jsonrpc\":\"2.0\"".to_vec();
        let mut cur = Cursor::new(buf);
        assert!(read_message(&mut cur).is_err());
    }

    /// CR-only line terminators (\r\n) are accepted (LSP-spec).
    #[test]
    fn framing_accepts_cr_lf_header_line() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"x","params":{}}"#;
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        buf.extend_from_slice(body);
        let mut cur = Cursor::new(buf);
        let m = read_message(&mut cur).unwrap().unwrap();
        assert_eq!(m.method, "x");
    }

    /// LF-only line terminators (`\n`) are also accepted (some clients
    /// emit those — be liberal in what you accept).
    #[test]
    fn framing_accepts_lf_only_header_line() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"y","params":{}}"#;
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(format!("Content-Length: {}\n\n", body.len()).as_bytes());
        buf.extend_from_slice(body);
        let mut cur = Cursor::new(buf);
        let m = read_message(&mut cur).unwrap().unwrap();
        assert_eq!(m.method, "y");
    }

    // ── Handler-level tests against an in-memory handler ──────────

    fn fake_handler() -> Handler {
        // Build a Handler with an empty registry/scopes — used to test
        // dispatch semantics without touching the filesystem.
        Handler::new_for_test(
            std::path::PathBuf::from("/tmp/ward-mcp-test"),
            Registry::new(),
            Vec::new(),
        )
    }

    #[test]
    fn handle_initialize_returns_server_info() {
        let h = fake_handler();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(42),
            method: "initialize".into(),
            params: json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0.0.1" },
            }),
        };
        let resp = h.handle(req);
        assert_eq!(resp.id, json!(42));
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "ward");
        assert!(result["serverInfo"]["version"].as_str().unwrap().len() > 0);
        assert_eq!(result["capabilities"]["tools"], json!({}));
        assert!(resp.error.is_none());
    }

    #[test]
    fn handle_tools_list_returns_exactly_five_tools() {
        let h = fake_handler();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(1),
            method: "tools/list".into(),
            params: json!({}),
        };
        let resp = h.handle(req);
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
        assert_eq!(tools.len(), 5);
        let names: Vec<&str> = tools.iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["scan_inventory", "move_item", "delete_item", "list_destinations", "audit_security"],
            "tool names must mirror CCO verbatim"
        );
    }

    #[test]
    fn tools_list_schemas_require_arguments_verbatim() {
        let h = fake_handler();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(1),
            method: "tools/list".into(),
            params: json!({}),
        };
        let resp = h.handle(req);
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();

        // move_item must require category, name, fromScopeId, toScopeId.
        let move_t = tools.iter().find(|t| t["name"] == "move_item").unwrap();
        let required: Vec<&str> = move_t["inputSchema"]["required"]
            .as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(required, vec!["category", "name", "fromScopeId", "toScopeId"]);

        // category enum must list exactly the actionable categories.
        let mut enum_vals: Vec<String> = move_t["inputSchema"]["properties"]["category"]["enum"]
            .as_array().unwrap().iter()
            .map(|v| v.as_str().unwrap().to_string()).collect();
        enum_vals.sort();
        let mut expected: Vec<String> = ACTION_CATEGORIES.iter().map(|s| s.to_string()).collect();
        expected.sort();
        assert_eq!(enum_vals, expected);

        // delete_item + list_destinations use scopeId (singular), not fromScopeId.
        for n in ["delete_item", "list_destinations"] {
            let t = tools.iter().find(|t| t["name"] == n).unwrap();
            let r: Vec<&str> = t["inputSchema"]["required"]
                .as_array().unwrap()
                .iter().map(|v| v.as_str().unwrap()).collect();
            assert_eq!(r, vec!["category", "name", "scopeId"], "{n} schema");
        }

        // scan_inventory + audit_security must have empty required.
        for n in ["scan_inventory", "audit_security"] {
            let t = tools.iter().find(|t| t["name"] == n).unwrap();
            let r = t["inputSchema"]["required"].as_array();
            assert!(
                r.as_ref().map(|a| a.is_empty()).unwrap_or(true),
                "{n} should have no required args; got {r:?}"
            );
        }
    }

    #[test]
    fn handle_unknown_method_is_method_not_found() {
        let h = fake_handler();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(7),
            method: "resources/list".into(),
            params: json!({}),
        };
        let resp = h.handle(req);
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, METHOD_NOT_FOUND);
        assert!(err.message.contains("resources/list"));
        assert_eq!(resp.id, json!(7));
    }

    #[test]
    fn handle_unknown_tool_is_method_not_found() {
        let h = fake_handler();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(8),
            method: "tools/call".into(),
            params: json!({ "name": "warp_drive", "arguments": {} }),
        };
        let resp = h.handle(req);
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, METHOD_NOT_FOUND);
        assert!(err.message.contains("warp_drive"));
    }

    #[test]
    fn handle_move_item_with_bogus_args_returns_invalid_params() {
        let h = fake_handler();
        // Missing required `toScopeId`.
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(9),
            method: "tools/call".into(),
            params: json!({
                "name": "move_item",
                "arguments": {
                    "category": "skill",
                    "name": "foo",
                    "fromScopeId": "global",
                },
            }),
        };
        let resp = h.handle(req);
        let err = resp.error.unwrap();
        assert_eq!(err.code, INVALID_PARAMS);
        assert!(err.message.contains("toScopeId"));
    }

    #[test]
    fn handle_move_item_with_unknown_category_returns_invalid_params() {
        let h = fake_handler();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(10),
            method: "tools/call".into(),
            params: json!({
                "name": "move_item",
                "arguments": {
                    "category": "config",
                    "name": "x",
                    "fromScopeId": "global",
                    "toScopeId": "global",
                },
            }),
        };
        let resp = h.handle(req);
        let err = resp.error.unwrap();
        assert_eq!(err.code, INVALID_PARAMS);
        assert!(err.message.contains("config"));
    }

    #[test]
    fn handle_list_destinations_for_unknown_category_returns_invalid_params() {
        let h = fake_handler();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(11),
            method: "tools/call".into(),
            params: json!({
                "name": "list_destinations",
                "arguments": { "category": "config", "name": "x", "scopeId": "global" },
            }),
        };
        let resp = h.handle(req);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn handle_notifications_returns_empty_result_no_panic() {
        let h = fake_handler();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Value::Null,
            method: "notifications/initialized".into(),
            params: json!({}),
        };
        let resp = h.handle(req);
        // Notifications are fire-and-forget; we respond with an empty result.
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn handle_scan_inventory_with_empty_registry_returns_tool_error() {
        // Build a handler with no Claude adapter registered so scan_impl fails.
        let h = fake_handler();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(12),
            method: "tools/call".into(),
            params: json!({ "name": "scan_inventory", "arguments": {} }),
        };
        let resp = h.handle(req);
        // scan_impl returns HarnessUnavailable → surfaced as tool error (isError: true).
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], json!(true));
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("claude") || text.contains("harness"));
    }

    #[test]
    fn category_enum_is_exhaustive_per_claude_ops() {
        // ACTION_CATEGORIES must contain every category ClaudeOps accepts for
        // move_item + delete_item. Drift here breaks tool callers that
        // expect a stable enum.
        let expected: &[&str] = &[
            "memory", "skill", "mcp", "command", "agent",
            "plan", "rule", "session",
        ];
        for c in expected {
            assert!(Handler::is_known_category(c), "missing category: {c}");
        }
    }
}
