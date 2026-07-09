//! Streaming JSONL parser for Claude Code **and** Codex CLI session files.
//!
//! Each line of a session `.jsonl` is a JSON object. The format is
//! auto-detected per line: a top-level `payload` object means a Codex
//! `rollout-*.jsonl` line (`response_item` / `event_msg` / `session_meta`);
//! otherwise it's a Claude Code line whose top-level `type` discriminates
//! the record.
//!
//! For Claude the interesting records are `user`, `assistant`, `ai-title`,
//! `system`, and `queue-operation`; everything else (e.g. `attachment`,
//! `progress`, `pr-link`, `file-history-snapshot`, `custom-title`) is
//! preserved as `Other`. For Codex the transcript comes from
//! `response_item` payloads (message / reasoning / function_call /
//! function_call_output); `event_msg` records duplicate them and are
//! preserved as `Other`.
//!
//! User/assistant turns are arrays of typed **content blocks** — plain
//! text is only one kind. `ContentBlock` models them all (text, thinking,
//! tool_use, tool_result, image) so the viewer can render tool calls,
//! results, and reasoning distinctly instead of collapsing everything to a
//! single (often empty) string.
//!
//! Parsing streams the file line-by-line via `BufReader::lines()` so
//! 100MB+ session files do not blow up memory. Single-line parse is
//! exposed as `parse_line` for tests and reuse.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::WardError;

// ── Wire model ─────────────────────────────────────────────────────────

/// Per-message usage block. Token counts come from the assistant
/// records; `cache_read` / `cache_write` may be absent for older
/// Claude Code versions, so both are optional.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<u64>,
}

/// A single content block inside a `user`/`assistant` message. Real
/// Claude Code (and Codex) messages are arrays of typed blocks — plain
/// text is only one of them. Modelling every block type lets the viewer
/// render tool calls, tool results, and reasoning distinctly instead of
/// collapsing everything to a single (often empty) string.
///
/// Internally tagged on `type` (camelCase) so the frontend can
/// discriminate: `text`, `thinking`, `toolUse`, `toolResult`, `image`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ContentBlock {
    /// A plain assistant/user text block (`{"type":"text","text":"…"}`).
    Text { text: String },
    /// Assistant chain-of-thought (`{"type":"thinking","thinking":"…"}`)
    /// or Codex `reasoning` summary. The text lives in `.thinking` /
    /// `.summary[].text`, never a top-level `.text`.
    Thinking { text: String },
    /// A tool invocation (`{"type":"tool_use","name":"…","input":{…}}`) or
    /// Codex `function_call`. No text of its own — identity is the tool
    /// name plus a short one-line summary of the salient input.
    ToolUse { name: String, input_summary: String },
    /// A tool's result (`{"type":"tool_result","content":…}`) or Codex
    /// `function_call_output`. `.content` may be a string OR an array of
    /// text blocks on disk — both are flattened into this string.
    ToolResult { content: String },
    /// An image block — rendered as a `[image]` placeholder (the base64
    /// payload is never surfaced).
    Image,
}

/// A single JSONL line, classified by `type`. User/Assistant records
/// carry the structured `blocks` (so the viewer can render tool calls,
/// results, and reasoning distinctly) plus a derived flattened `content`
/// string (kept for search / cost / distill compatibility).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionRecord {
    /// `{"type":"user","message":{"role":"user","content":...},"timestamp":"..."}`
    User {
        content: String,
        #[serde(default)]
        blocks: Vec<ContentBlock>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ts: Option<String>,
    },
    /// `{"type":"assistant","message":{"role":"assistant","content":[...],"model":"...","usage":{...}}}`
    Assistant {
        content: String,
        #[serde(default)]
        blocks: Vec<ContentBlock>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ts: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    /// `{"type":"system","subtype":"compact_boundary","...":"..."}`
    System {
        subtype: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    /// `{"type":"ai-title","aiTitle":"..."}`
    AiTitle { title: String },
    /// `{"type":"summary","summary":"...","leafUuid":"..."}` — Claude Code's
    /// auto-generated conversation title. Previously discarded as `Other`.
    Summary {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        leaf_uuid: Option<String>,
    },
    /// `{"type":"queue-operation",...}`
    QueueOperation { enqueue: bool },
    /// Anything we don't surface to the UI — preserved so the record
    /// count is honest about what was scanned.
    Other {
        #[serde(rename = "recordType")]
        record_type: String,
    },
}

/// A parsed session file. The `records` vec holds every JSONL line that
/// parsed (malformed lines are skipped silently — they happen when CC
/// crashes mid-write).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub session_id: String,
    pub records: Vec<SessionRecord>,
}

impl Conversation {
    fn empty(session_id: String) -> Self {
        Self { session_id, records: Vec::new() }
    }
}

// ── Public API ─────────────────────────────────────────────────────────

/// Stream-parse a session JSONL file. The whole file is *not* loaded
/// into memory — we read line-by-line via `BufReader::lines()`. Lines
/// that fail to parse are skipped (CC occasionally writes a half-line
/// on crash); the returned `Conversation` only contains well-formed
/// records.
pub fn parse_file(path: &Path) -> Result<Conversation, WardError> {
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let file = File::open(path)?;
    let reader = BufReader::with_capacity(64 * 1024, file);
    let mut conv = Conversation::empty(session_id);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue, // skip encoding errors at the byte level
        };
        if let Some(rec) = parse_line(&line) {
            conv.records.push(rec);
        }
    }
    Ok(conv)
}

/// Parse a single JSONL line. Returns `None` for blank lines and
/// unparseable input. Exposed for unit tests.
pub fn parse_line(line: &str) -> Option<SessionRecord> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_str(trimmed).ok()?;
    Some(classify(value))
}

// ── Classifier ─────────────────────────────────────────────────────────

fn classify(v: Value) -> SessionRecord {
    let obj = match v.as_object() {
        Some(o) => o,
        None => {
            return SessionRecord::Other { record_type: "non-object".into() }
        }
    };
    let ts = obj
        .get("timestamp")
        .and_then(|t| t.as_str())
        .map(str::to_string);
    // Codex rollout lines carry a top-level `payload` object (and no
    // Claude-style `message` envelope). Route them to the Codex classifier.
    if obj.contains_key("payload") {
        return classify_codex(obj, ts);
    }
    let record_type = obj
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("unknown")
        .to_string();
    let message = obj.get("message").and_then(|m| m.as_object());
    let role = message
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        .unwrap_or("");
    let content = message.and_then(|m| m.get("content"));

    match (record_type.as_str(), role) {
        ("user", "user") => {
            let blocks = parse_claude_blocks(content);
            SessionRecord::User { content: flatten_blocks(&blocks), blocks, ts }
        }
        ("assistant", "assistant") => {
            let blocks = parse_claude_blocks(content);
            SessionRecord::Assistant {
                content: flatten_blocks(&blocks),
                blocks,
                model: message
                    .and_then(|m| m.get("model"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                ts,
                usage: message.and_then(|m| m.get("usage")).and_then(parse_usage),
            }
        }
        ("system", _) => {
            let subtype = obj
                .get("subtype")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let summary = obj
                .get("summary")
                .and_then(|s| s.as_str())
                .map(str::to_string);
            SessionRecord::System { subtype, summary }
        }
        ("ai-title", _) => SessionRecord::AiTitle {
            title: obj
                .get("aiTitle")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string(),
        },
        ("queue-operation", _) => SessionRecord::QueueOperation {
            enqueue: obj
                .get("enqueue")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        },
        ("summary", _) => SessionRecord::Summary {
            text: obj
                .get("summary")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            leaf_uuid: obj
                .get("leafUuid")
                .and_then(|s| s.as_str())
                .map(str::to_string),
        },
        _ => SessionRecord::Other { record_type },
    }
}

// ── Codex classifier ───────────────────────────────────────────────────
//
// Codex `rollout-*.jsonl` lines look like
//   {"type":"response_item"|"event_msg"|"session_meta","timestamp":"…",
//    "payload":{ … }}
// There is no `message`/`role` envelope. The canonical transcript lives
// in `response_item` payloads:
//   - message            → user/assistant text (payload.content[].text)
//   - reasoning          → thinking (payload.summary[].text)
//   - function_call      → tool call (name + arguments)
//   - function_call_output → tool result (output)
// `event_msg` records (agent_message/user_message/token_count/…) DUPLICATE
// the response_item transcript, so mapping them to `Other` avoids doubling
// every turn.

fn classify_codex(obj: &serde_json::Map<String, Value>, ts: Option<String>) -> SessionRecord {
    let record_type = obj.get("type").and_then(|t| t.as_str()).unwrap_or("unknown");
    let payload = obj.get("payload").and_then(|p| p.as_object());
    let ptype = payload
        .and_then(|p| p.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("");

    // Only `response_item` payloads are conversation turns. Everything else
    // (event_msg duplicates, session_meta, turn_context, compacted) is
    // envelope/telemetry — preserved as Other for an honest record count.
    if record_type != "response_item" {
        return SessionRecord::Other { record_type: record_type.to_string() };
    }

    match ptype {
        "message" => {
            let role = payload
                .and_then(|p| p.get("role"))
                .and_then(|r| r.as_str())
                .unwrap_or("");
            let blocks = parse_codex_message_blocks(payload.and_then(|p| p.get("content")));
            let content = flatten_blocks(&blocks);
            // role is user/assistant/developer; only assistant is the model.
            if role == "assistant" {
                SessionRecord::Assistant { content, blocks, model: None, ts, usage: None }
            } else {
                SessionRecord::User { content, blocks, ts }
            }
        }
        "reasoning" => {
            let text = codex_reasoning_text(payload);
            let blocks = if text.is_empty() {
                Vec::new()
            } else {
                vec![ContentBlock::Thinking { text: text.clone() }]
            };
            SessionRecord::Assistant { content: text, blocks, model: None, ts, usage: None }
        }
        "function_call" | "custom_tool_call" => {
            let name = payload
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            // `function_call` carries a JSON-string `arguments`;
            // `custom_tool_call` carries a raw-string `input`.
            let raw = payload.and_then(|p| p.get("arguments").or_else(|| p.get("input")));
            let input_summary = summarize_codex_args(raw);
            let block = ContentBlock::ToolUse { name: name.clone(), input_summary: input_summary.clone() };
            SessionRecord::Assistant {
                content: tool_use_label(&name, &input_summary),
                blocks: vec![block],
                model: None,
                ts,
                usage: None,
            }
        }
        "function_call_output" | "custom_tool_call_output" => {
            let output = payload
                .and_then(|p| p.get("output"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            SessionRecord::User {
                content: output.clone(),
                blocks: vec![ContentBlock::ToolResult { content: output }],
                ts,
            }
        }
        // Non-turn response items (web_search_call, ghost_snapshot, …).
        _ => SessionRecord::Other { record_type: format!("response_item:{ptype}") },
    }
}

/// Codex `message.content` is an array of `input_text` / `output_text`
/// blocks (both carry `.text`); a bare string is tolerated too.
fn parse_codex_message_blocks(content: Option<&Value>) -> Vec<ContentBlock> {
    match content {
        Some(Value::String(s)) => vec![ContentBlock::Text { text: s.clone() }],
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| {
                b.get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| ContentBlock::Text { text: s.to_string() })
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Codex reasoning text lives in `summary[].text` (`.content` is usually
/// null / encrypted). Falls back to `content[].text` if present.
fn codex_reasoning_text(payload: Option<&serde_json::Map<String, Value>>) -> String {
    let p = match payload {
        Some(p) => p,
        None => return String::new(),
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(arr) = p.get("summary").and_then(|v| v.as_array()) {
        for b in arr {
            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                parts.push(t.to_string());
            }
        }
    }
    if parts.is_empty() {
        if let Some(arr) = p.get("content").and_then(|v| v.as_array()) {
            for b in arr {
                if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                    parts.push(t.to_string());
                }
            }
        }
    }
    parts.join("\n")
}

/// Summarize a Codex tool call's arguments. `function_call.arguments` is a
/// JSON *string* — parse it and reuse `summarize_input`; `custom_tool_call.
/// input` is a raw string — collapse and truncate it.
fn summarize_codex_args(raw: Option<&Value>) -> String {
    match raw {
        Some(Value::String(s)) => {
            if let Ok(v) = serde_json::from_str::<Value>(s) {
                let sum = summarize_input(Some(&v));
                if !sum.is_empty() {
                    return sum;
                }
            }
            one_line_truncate(s, 80)
        }
        Some(v) => summarize_input(Some(v)),
        None => String::new(),
    }
}

/// Parse a Claude `message.content` value into structured blocks. A
/// bare string becomes a single `Text` block; an array is walked block
/// by block. This is the replacement for the old lossy `extract_text`,
/// which only ever read blocks with a top-level `.text` — silently
/// dropping every `tool_result`, `tool_use`, and `thinking` block (the
/// vast majority of real turns).
fn parse_claude_blocks(content: Option<&Value>) -> Vec<ContentBlock> {
    match content {
        Some(Value::String(s)) => vec![ContentBlock::Text { text: s.clone() }],
        Some(Value::Array(arr)) => arr.iter().filter_map(parse_claude_block).collect(),
        _ => Vec::new(),
    }
}

/// Classify a single Claude content block. Returns `None` for a block we
/// can't turn into anything meaningful (e.g. a `text` block with no
/// `text` field).
fn parse_claude_block(b: &Value) -> Option<ContentBlock> {
    let btype = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match btype {
        "text" => b
            .get("text")
            .and_then(|t| t.as_str())
            .map(|s| ContentBlock::Text { text: s.to_string() }),
        "thinking" => Some(ContentBlock::Thinking {
            text: b
                .get("thinking")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "tool_use" => {
            let name = b
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            Some(ContentBlock::ToolUse {
                name,
                input_summary: summarize_input(b.get("input")),
            })
        }
        "tool_result" => Some(ContentBlock::ToolResult {
            content: tool_result_text(b.get("content")),
        }),
        "image" => Some(ContentBlock::Image),
        // Unknown block: salvage a `.text` if present, else drop it.
        _ => b
            .get("text")
            .and_then(|t| t.as_str())
            .map(|s| ContentBlock::Text { text: s.to_string() }),
    }
}

/// A `tool_result` block's `.content` is a String on most turns but an
/// array of `{type:"text",text:…}` (and/or image) blocks on others.
/// Both collapse to a single newline-joined string.
fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()).map(str::to_string))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Flatten structured blocks into a single searchable/derivable string.
/// This is the `content` field kept for search, cost, and distill
/// compatibility. Empty fragments are skipped so a lone empty text block
/// still flattens to "" (and distill still drops it).
fn flatten_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .map(block_display_text)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The single-line/inline textual representation of one block, used both
/// for the flattened `content` and as a fallback in the viewer.
fn block_display_text(b: &ContentBlock) -> String {
    match b {
        ContentBlock::Text { text } => text.clone(),
        ContentBlock::Thinking { text } => text.clone(),
        ContentBlock::ToolUse { name, input_summary } => tool_use_label(name, input_summary),
        ContentBlock::ToolResult { content } => content.clone(),
        ContentBlock::Image => "[image]".to_string(),
    }
}

/// Compact label for a tool call: `name: summary` (e.g. `Bash: git
/// status`) or just `name` when there is no salient input to show.
fn tool_use_label(name: &str, input_summary: &str) -> String {
    if input_summary.is_empty() {
        name.to_string()
    } else {
        format!("{name}: {input_summary}")
    }
}

/// Priority order of input keys to surface as a tool-call summary. Covers
/// the common Claude Code + Codex tools (Bash `command`, Read/Edit/Write
/// `file_path`, Grep/Glob `pattern`, WebFetch `url`, Skill `command`, …).
const INPUT_SUMMARY_KEYS: &[&str] = &[
    "command",
    "file_path",
    "path",
    "pattern",
    "query",
    "url",
    "skill",
    "description",
    "prompt",
    "name",
    "text",
    "content",
];

/// Summarize a `tool_use.input` object into a short one-line string. We
/// pick the first salient key (by `INPUT_SUMMARY_KEYS` priority), falling
/// back to the first string value, and truncate to keep the row compact.
fn summarize_input(input: Option<&Value>) -> String {
    match input {
        Some(Value::Object(map)) => {
            for k in INPUT_SUMMARY_KEYS {
                if let Some(s) = map.get(*k).and_then(|v| v.as_str()) {
                    return one_line_truncate(s, 80);
                }
            }
            // Fall back to the first string-valued field, if any.
            map.values()
                .find_map(|v| v.as_str())
                .map(|s| one_line_truncate(s, 80))
                .unwrap_or_default()
        }
        Some(Value::String(s)) => one_line_truncate(s, 80),
        _ => String::new(),
    }
}

/// Collapse a value to a single trimmed line and truncate to `max`
/// characters (char-boundary safe), appending an ellipsis when clipped.
fn one_line_truncate(s: &str, max: usize) -> String {
    let one: String = s.chars().map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c }).collect();
    let one = one.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > max {
        let clipped: String = one.chars().take(max).collect();
        format!("{clipped}…")
    } else {
        one
    }
}

fn parse_usage(v: &Value) -> Option<Usage> {
    let obj = v.as_object()?;
    let input = obj.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output = obj.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let cache_read = obj.get("cache_read_input_tokens").and_then(|v| v.as_u64());
    let cache_write = obj.get("cache_creation_input_tokens").and_then(|v| v.as_u64());
    Some(Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read,
        cache_write,
    })
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_session(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn parse_line_user_string_content() {
        let line = r#"{"type":"user","message":{"role":"user","content":"hello"},"timestamp":"2026-01-01T00:00:00Z"}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::User { content, ts, .. } => {
                assert_eq!(content, "hello");
                assert_eq!(ts.as_deref(), Some("2026-01-01T00:00:00Z"));
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_user_array_content_joins_text() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"alpha"},{"type":"text","text":"beta"}]}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::User { content, .. } => {
                assert_eq!(content, "alpha\nbeta");
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_assistant_with_usage_and_model() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}],"model":"claude-sonnet-4-5","usage":{"input_tokens":120,"output_tokens":30,"cache_read_input_tokens":1000,"cache_creation_input_tokens":500}}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::Assistant { content, model, usage, .. } => {
                assert_eq!(content, "hi");
                assert_eq!(model.as_deref(), Some("claude-sonnet-4-5"));
                let u = usage.unwrap();
                assert_eq!(u.input_tokens, 120);
                assert_eq!(u.output_tokens, 30);
                assert_eq!(u.cache_read, Some(1000));
                assert_eq!(u.cache_write, Some(500));
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_assistant_without_usage() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"x"}],"model":"claude-haiku-4-5"}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::Assistant { content, usage, .. } => {
                assert_eq!(content, "x");
                assert!(usage.is_none());
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_ai_title() {
        let line = r#"{"type":"ai-title","aiTitle":"Plan 07 build"}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::AiTitle { title } => assert_eq!(title, "Plan 07 build"),
            other => panic!("expected AiTitle, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_system_compact_boundary() {
        let line = r#"{"type":"system","subtype":"compact_boundary","summary":"compacted"}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::System { subtype, summary } => {
                assert_eq!(subtype, "compact_boundary");
                assert_eq!(summary.as_deref(), Some("compacted"));
            }
            other => panic!("expected System, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_queue_operation() {
        let line = r#"{"type":"queue-operation","enqueue":true}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::QueueOperation { enqueue } => assert!(enqueue),
            other => panic!("expected QueueOperation, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_attachment_becomes_other() {
        let line = r#"{"type":"attachment","id":"abc"}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::Other { record_type } => assert_eq!(record_type, "attachment"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn parse_line_blank_returns_none() {
        assert!(parse_line("").is_none());
        assert!(parse_line("   \n").is_none());
    }

    #[test]
    fn parse_line_malformed_returns_none() {
        assert!(parse_line("{not json").is_none());
    }

    #[test]
    fn parse_file_streams_and_skips_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let body = "\
{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n\
{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"hi\"}],\"model\":\"claude-opus-4-1\"}}\n\
{this line is broken\n\
{\"type\":\"ai-title\",\"aiTitle\":\"t\"}\n\
\n\
{\"type\":\"queue-operation\",\"enqueue\":true}\n";
        let p = write_session(dir.path(), "abc.jsonl", body);
        let conv = parse_file(&p).unwrap();
        assert_eq!(conv.session_id, "abc");
        assert_eq!(conv.records.len(), 4, "malformed + blank line skipped");
        assert!(matches!(conv.records[0], SessionRecord::User { .. }));
        assert!(matches!(conv.records[1], SessionRecord::Assistant { .. }));
        assert!(matches!(conv.records[2], SessionRecord::AiTitle { .. }));
        assert!(matches!(conv.records[3], SessionRecord::QueueOperation { .. }));
    }

    #[test]
    fn parse_file_extracts_session_id_from_filename() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "abc-123-def.jsonl", "{\"type\":\"ai-title\",\"aiTitle\":\"x\"}\n");
        let conv = parse_file(&p).unwrap();
        assert_eq!(conv.session_id, "abc-123-def");
    }

    #[test]
    fn parse_file_handles_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope.jsonl");
        assert!(parse_file(&p).is_err());
    }

    #[test]
    fn conversation_serializes_camel_case() {
        let c = Conversation {
            session_id: "x".into(),
            records: vec![SessionRecord::User {
                content: "hi".into(),
                blocks: vec![ContentBlock::Text { text: "hi".into() }],
                ts: None,
            }],
        };
        let j = serde_json::to_string(&c).unwrap();
        assert!(j.contains("\"sessionId\":\"x\""));
        assert!(j.contains("\"records\""));
        assert!(j.contains("\"kind\":\"user\""));
    }

    // ── Structured content-block golden tests (Commit A) ──────────────
    // These use the REAL on-disk Claude Code shapes: user turns are
    // `tool_result` blocks (string OR array content), assistant turns are
    // `thinking` + `tool_use` (+ optional `text`) blocks. None of these
    // carry a top-level `.text`, so the old `extract_text` collapsed them
    // to "" — the "(empty)" bug. Each asserts the flattened `content` now
    // carries the real text AND that structured `blocks` are populated.

    #[test]
    fn parse_user_tool_result_string_content() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01ABC","content":"1\tfirst line\n2\tsecond line"}]}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::User { content, blocks, .. } => {
                assert_eq!(content, "1\tfirst line\n2\tsecond line");
                assert_eq!(blocks, vec![ContentBlock::ToolResult {
                    content: "1\tfirst line\n2\tsecond line".into(),
                }]);
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[test]
    fn parse_user_tool_result_array_content() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_02DEF","content":[{"type":"text","text":"line one"},{"type":"text","text":"line two"}]}]}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::User { content, blocks, .. } => {
                assert_eq!(content, "line one\nline two");
                assert_eq!(blocks, vec![ContentBlock::ToolResult {
                    content: "line one\nline two".into(),
                }]);
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[test]
    fn parse_assistant_thinking_block() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Let me consider the options carefully.","signature":"EqoBd3aE"}],"model":"claude-opus-4-8"}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::Assistant { content, blocks, model, .. } => {
                assert_eq!(content, "Let me consider the options carefully.");
                assert_eq!(model.as_deref(), Some("claude-opus-4-8"));
                assert_eq!(blocks, vec![ContentBlock::Thinking {
                    text: "Let me consider the options carefully.".into(),
                }]);
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn parse_assistant_tool_use_renders_name() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_03GHI","name":"Bash","input":{"command":"git status","description":"check status"}}],"model":"claude-opus-4-8"}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::Assistant { content, blocks, .. } => {
                // Compact label = name + first salient input value.
                assert_eq!(content, "Bash: git status");
                assert_eq!(blocks, vec![ContentBlock::ToolUse {
                    name: "Bash".into(),
                    input_summary: "git status".into(),
                }]);
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn parse_assistant_mixed_thinking_tool_use_text() {
        // A real-shaped assistant line: thinking + tool_use + text together.
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Planning the approach.","signature":"sig"},{"type":"tool_use","id":"toolu_04","name":"Skill","input":{"command":"brainstorming"}},{"type":"text","text":"Done."}],"model":"claude-opus-4-8"}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::Assistant { content, blocks, .. } => {
                assert!(!content.is_empty(), "mixed assistant turn must not be empty");
                assert_eq!(content, "Planning the approach.\nSkill: brainstorming\nDone.");
                assert_eq!(blocks.len(), 3);
                assert!(matches!(blocks[0], ContentBlock::Thinking { .. }));
                assert!(matches!(blocks[1], ContentBlock::ToolUse { .. }));
                assert!(matches!(blocks[2], ContentBlock::Text { .. }));
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn parse_user_image_block_placeholder() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}},{"type":"text","text":"see attached"}]}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::User { content, blocks, .. } => {
                assert_eq!(content, "[image]\nsee attached");
                assert_eq!(blocks, vec![
                    ContentBlock::Image,
                    ContentBlock::Text { text: "see attached".into() },
                ]);
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[test]
    fn parse_user_string_content_yields_single_text_block() {
        let line = r#"{"type":"user","message":{"role":"user","content":"plain prompt"}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::User { content, blocks, .. } => {
                assert_eq!(content, "plain prompt");
                assert_eq!(blocks, vec![ContentBlock::Text { text: "plain prompt".into() }]);
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[test]
    fn content_block_serializes_camel_case_internally_tagged() {
        let b = ContentBlock::ToolUse { name: "Read".into(), input_summary: "/tmp/x".into() };
        let j = serde_json::to_string(&b).unwrap();
        assert!(j.contains("\"type\":\"toolUse\""), "got {j}");
        assert!(j.contains("\"name\":\"Read\""));
        assert!(j.contains("\"inputSummary\":\"/tmp/x\""));
        let img = serde_json::to_string(&ContentBlock::Image).unwrap();
        assert_eq!(img, "{\"type\":\"image\"}");
    }

    // ── Codex transcript golden tests (Commit C) ─────────────────────
    // Codex rollout lines carry a top-level `payload` (no `message`/`role`
    // envelope). Text lives at payload.content[].text (input_text/
    // output_text); reasoning at payload.summary[].text; tools at
    // function_call / function_call_output. Shapes are verbatim from real
    // ~/.codex/sessions/**/rollout-*.jsonl files. Before the Codex path
    // these all classified as `Other` (transcript 100% empty).

    #[test]
    fn parse_codex_message_user() {
        let line = r#"{"type":"response_item","timestamp":"2025-12-29T11:18:03.838Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix the login bug please"}]}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::User { content, blocks, ts } => {
                assert_eq!(content, "Fix the login bug please");
                assert_eq!(blocks, vec![ContentBlock::Text { text: "Fix the login bug please".into() }]);
                assert_eq!(ts.as_deref(), Some("2025-12-29T11:18:03.838Z"));
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[test]
    fn parse_codex_message_assistant() {
        let line = r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Here is the fix you asked for."}]}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::Assistant { content, blocks, model, usage, .. } => {
                assert_eq!(content, "Here is the fix you asked for.");
                assert_eq!(blocks, vec![ContentBlock::Text { text: "Here is the fix you asked for.".into() }]);
                assert!(model.is_none());
                assert!(usage.is_none());
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn parse_codex_message_developer_role_is_user() {
        // Codex injects a `developer` role for instruction messages; it's an
        // input turn, so it maps to User (not assistant).
        let line = r#"{"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"AGENTS.md instructions"}]}}"#;
        let rec = parse_line(line).unwrap();
        assert!(matches!(rec, SessionRecord::User { .. }), "developer role -> User, got {rec:?}");
    }

    #[test]
    fn parse_codex_reasoning() {
        let line = r#"{"type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"Planning the approach carefully."}],"content":null,"encrypted_content":"gAAAAABpUmO0"}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::Assistant { content, blocks, .. } => {
                assert_eq!(content, "Planning the approach carefully.");
                assert_eq!(blocks, vec![ContentBlock::Thinking { text: "Planning the approach carefully.".into() }]);
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn parse_codex_function_call() {
        // `arguments` is a JSON *string*; we parse it and surface `command`.
        let line = r#"{"type":"response_item","payload":{"type":"function_call","name":"shell_command","arguments":"{\"command\":\"ls -la\",\"workdir\":\"/tmp\"}","call_id":"call_fjvl"}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::Assistant { content, blocks, .. } => {
                assert_eq!(content, "shell_command: ls -la");
                assert_eq!(blocks, vec![ContentBlock::ToolUse {
                    name: "shell_command".into(),
                    input_summary: "ls -la".into(),
                }]);
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn parse_codex_function_call_output() {
        let line = r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"call_fjvl","output":"Exit code: 0\nOutput:\nfile.txt"}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::User { content, blocks, .. } => {
                assert_eq!(content, "Exit code: 0\nOutput:\nfile.txt");
                assert_eq!(blocks, vec![ContentBlock::ToolResult { content: "Exit code: 0\nOutput:\nfile.txt".into() }]);
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[test]
    fn parse_codex_custom_tool_call_apply_patch() {
        // custom_tool_call `input` is a raw (non-JSON) string.
        let line = r#"{"type":"response_item","payload":{"type":"custom_tool_call","status":"completed","call_id":"c1","name":"apply_patch","input":"*** Begin Patch\n*** Update File: manifest.json"}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::Assistant { blocks, content, .. } => {
                assert!(content.starts_with("apply_patch: "), "got {content}");
                assert!(matches!(&blocks[..], [ContentBlock::ToolUse { name, .. }] if name == "apply_patch"));
            }
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[test]
    fn parse_codex_event_msg_is_meta_not_duplicate() {
        // event_msg records DUPLICATE the response_item transcript
        // (agent_message, user_message, token_count, …). They map to Other
        // so the transcript is not doubled.
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{}}}"#;
        let rec = parse_line(line).unwrap();
        match rec {
            SessionRecord::Other { record_type } => assert_eq!(record_type, "event_msg"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn parse_codex_session_meta_is_other() {
        let line = r#"{"type":"session_meta","payload":{"id":"abc","cwd":"/x","cli_version":"0.77.0"}}"#;
        let rec = parse_line(line).unwrap();
        assert!(matches!(rec, SessionRecord::Other { .. }), "session_meta -> Other, got {rec:?}");
    }

    #[test]
    fn parse_file_codex_rollout_populates_transcript() {
        // End-to-end: a mixed Codex rollout yields non-empty user/assistant
        // turns (the regression this fixes: Codex was 100% empty).
        let dir = tempfile::tempdir().unwrap();
        let body = "\
{\"type\":\"session_meta\",\"payload\":{\"id\":\"x\",\"cwd\":\"/p\"}}\n\
{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"build it\"}]}}\n\
{\"type\":\"response_item\",\"payload\":{\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"think think\"}],\"content\":null}}\n\
{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"shell_command\",\"arguments\":\"{\\\"command\\\":\\\"make\\\"}\",\"call_id\":\"c\"}}\n\
{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"c\",\"output\":\"done\"}}\n\
{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}\n\
{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"built ok\"}]}}\n";
        let p = write_session(dir.path(), "rollout-x.jsonl", body);
        let conv = parse_file(&p).unwrap();
        // Every user/assistant record carries non-empty content.
        let convo_turns: Vec<&SessionRecord> = conv.records.iter().filter(|r| matches!(r, SessionRecord::User { .. } | SessionRecord::Assistant { .. })).collect();
        assert_eq!(convo_turns.len(), 5, "user prompt + reasoning + tool call + tool result + assistant reply");
        for r in convo_turns {
            let c = match r {
                SessionRecord::User { content, .. } | SessionRecord::Assistant { content, .. } => content,
                _ => unreachable!(),
            };
            assert!(!c.trim().is_empty(), "codex turn must not be empty: {r:?}");
        }
    }

    #[test]
    fn usage_serializes_with_only_present_cache_fields() {
        let u = Usage {
            input_tokens: 10,
            output_tokens: 20,
            cache_read: Some(100),
            cache_write: None,
        };
        let j = serde_json::to_string(&u).unwrap();
        assert!(j.contains("\"inputTokens\":10"));
        assert!(j.contains("\"outputTokens\":20"));
        assert!(j.contains("\"cacheRead\":100"));
        assert!(!j.contains("cacheWrite"), "None cacheWrite must be omitted");
    }

    #[test]
    fn parse_line_summary_becomes_summary_record() {
        let line = r#"{"type":"summary","summary":"Refactor the auth flow","leafUuid":"abc-123"}"#;
        let rec = parse_line(line).expect("summary line parses");
        assert_eq!(
            rec,
            SessionRecord::Summary {
                text: "Refactor the auth flow".to_string(),
                leaf_uuid: Some("abc-123".to_string()),
            }
        );
    }
}