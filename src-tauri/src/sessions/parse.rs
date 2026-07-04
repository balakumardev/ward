//! Streaming JSONL parser for Claude Code session files.
//!
//! Each line of a session `.jsonl` is a JSON object whose top-level
//! `type` discriminates the record. The interesting records for the UI
//! are `user`, `assistant`, `ai-title`, `system`, and `queue-operation`;
//! everything else (e.g. `attachment`, `progress`, `pr-link`,
//! `file-history-snapshot`, `custom-title`) is preserved as `Other` so
//! the conversation viewer can still display its presence.
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

/// A single JSONL line, classified by `type`. The content fields are
/// pre-extracted (string or normalized text) so the UI can render them
/// without walking the raw `Value`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionRecord {
    /// `{"type":"user","message":{"role":"user","content":...},"timestamp":"..."}`
    User {
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ts: Option<String>,
    },
    /// `{"type":"assistant","message":{"role":"assistant","content":[...],"model":"...","usage":{...}}}`
    Assistant {
        content: String,
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
    let ts = obj
        .get("timestamp")
        .and_then(|t| t.as_str())
        .map(str::to_string);

    match (record_type.as_str(), role) {
        ("user", "user") => SessionRecord::User {
            content: extract_text(content),
            ts,
        },
        ("assistant", "assistant") => SessionRecord::Assistant {
            content: extract_text(content),
            model: message
                .and_then(|m| m.get("model"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            ts,
            usage: message.and_then(|m| m.get("usage")).and_then(parse_usage),
        },
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
        _ => SessionRecord::Other { record_type },
    }
}

/// Pull the textual content out of a `message.content` value. Strings
/// pass through; arrays of content blocks get joined by `\n` so the
/// UI can show them as a single block.
fn extract_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => {
            let mut out = String::new();
            for block in arr {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
            }
            out
        }
        _ => String::new(),
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
            SessionRecord::User { content, ts } => {
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
                ts: None,
            }],
        };
        let j = serde_json::to_string(&c).unwrap();
        assert!(j.contains("\"sessionId\":\"x\""));
        assert!(j.contains("\"records\""));
        assert!(j.contains("\"kind\":\"user\""));
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
}