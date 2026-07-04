//! Image trimming (Plan 07) — port of CCO's `trim-images.mjs`.
//!
//! Replaces image-bearing blocks inside any session JSONL line with a
//! `{type:"text", text:"[image redacted]"}` block. Surrounding text
//! structure is preserved verbatim (block arrays keep their non-image
//! blocks intact, and `[image redacted]` lands in the same slot).
//!
//! The trim function is exposed in two forms:
//!   - `trim_images(jsonl_content)` operates on the whole file's text.
//!     This is what `trim_file` writes through.
//!   - `trim_file(session_path)` reads, trims, writes back via a
//!     `RestoreInfo` so the Organizer can offer Undo.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::error::WardError;
use crate::model::RestoreInfo;

/// Replace image blocks in any `message.content` array with a
/// `[image redacted]` text block. Lines that don't carry any image
/// content are emitted verbatim (no JSON re-serialize round-trip).
pub fn trim_images(jsonl_content: &str) -> String {
    // Filter blank lines so trailing/leading/empty lines don't
    // introduce double-newlines after joining.
    let mut out = String::with_capacity(jsonl_content.len());
    let mut first = true;
    for line in jsonl_content.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !first {
            out.push('\n');
        }
        first = false;
        // Try to parse + redact; if anything goes wrong we keep the
        // line verbatim rather than corrupting the file.
        match try_redact_line(trimmed) {
            Some(redacted) => out.push_str(&redacted),
            None => out.push_str(trimmed),
        }
    }
    out
}

fn try_redact_line(line: &str) -> Option<String> {
    let mut v: Value = serde_json::from_str(line).ok()?;
    let changed = redact_value(&mut v);
    if changed {
        Some(serde_json::to_string(&v).ok()?)
    } else {
        // Nothing changed — return None so the caller keeps the
        // original line text (preserves trailing whitespace etc).
        None
    }
}

/// Walk `v` looking for `message.content` arrays that contain image
/// blocks. Returns true if any mutation was applied.
fn redact_value(v: &mut Value) -> bool {
    let mut changed = false;
    if let Some(message) = v.get_mut("message") {
        if let Some(content) = message.get_mut("content") {
            if let Some(arr) = content.as_array_mut() {
                for block in arr.iter_mut() {
                    if redact_block(block) {
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

/// If `block` is an image (top-level or nested inside a tool_result
/// content array), replace it with a `[image redacted]` text block.
fn redact_block(block: &mut Value) -> bool {
    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if block_type == "image" || is_base64_source(block) {
        *block = serde_json::json!({ "type": "text", "text": "[image redacted]" });
        return true;
    }
    if block_type == "tool_result" {
        if let Some(content) = block.get_mut("content") {
            if let Some(arr) = content.as_array_mut() {
                let mut any = false;
                for child in arr.iter_mut() {
                    let child_type = child.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if child_type == "image" || is_base64_source(child) {
                        *child = serde_json::json!({ "type": "text", "text": "[image redacted]" });
                        any = true;
                    }
                }
                return any;
            }
        }
    }
    false
}

fn is_base64_source(block: &Value) -> bool {
    block.get("source")
        .and_then(|s| s.get("type"))
        .and_then(|t| t.as_str())
        .map(|t| t == "base64")
        .unwrap_or(false)
}

/// Read `session_path`, redact image blocks, write the result back.
/// Returns a `RestoreInfo` capturing the original bytes verbatim so
/// the Organizer can offer Undo via the existing `restore` pipeline.
pub fn trim_file(session_path: &Path) -> Result<RestoreInfo, WardError> {
    let abs = session_path.to_path_buf();
    let original_bytes = fs::read(&abs)?;
    let original_text = String::from_utf8(original_bytes.clone())
        .map_err(|e| WardError::NotFound(format!("session is not valid UTF-8: {e}")))?;
    let trimmed = trim_images(&original_text);
    fs::write(&abs, trimmed.as_bytes())?;
    Ok(RestoreInfo {
        kind: "file".into(),
        original_path: abs.display().to_string(),
        current_path: Some(abs.display().to_string()),
        backup_bytes: Some(original_bytes),
        mcp_entry: None,
        mcp_key: None,
        mcp_parent_key: None,
        mcp_scope: None,
    })
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn trims_top_level_image_block() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}},{"type":"text","text":"after"}]}}"#;
        let out = trim_images(line);
        assert!(out.contains("[image redacted]"));
        assert!(out.contains("\"text\":\"after\""));
        assert!(!out.contains("AAAA"), "base64 data must be removed");
    }

    #[test]
    fn trims_nested_image_in_tool_result() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"image","source":{"type":"base64","data":"BBBB"}},{"type":"text","text":"ok"}]}]}}"#;
        let out = trim_images(line);
        assert!(out.contains("[image redacted]"));
        assert!(out.contains("\"text\":\"ok\""));
        assert!(!out.contains("BBBB"));
    }

    #[test]
    fn preserves_non_image_blocks() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"keep"}]}}"#;
        let out = trim_images(line);
        assert_eq!(out, line, "non-image line must round-trip identically");
    }

    #[test]
    fn handles_multiline_jsonl() {
        let body = "\
{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"plain text\"}}\n\
{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"image\",\"source\":{\"type\":\"base64\",\"data\":\"ZZZZ\"}}]}}\n\
{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n";
        let out = trim_images(body);
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("plain text"));
        assert!(lines[1].contains("[image redacted]"));
        assert!(!lines[1].contains("ZZZZ"));
        assert!(lines[2].contains("\"text\":\"ok\""));
    }

    #[test]
    fn invalid_json_line_passes_through_untouched() {
        let body = "not json at all\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"image\",\"source\":{\"type\":\"base64\",\"data\":\"QQ\"}}]}}";
        let out = trim_images(body);
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(lines[0], "not json at all");
        assert!(lines[1].contains("[image redacted]"));
    }

    #[test]
    fn blank_lines_skipped() {
        let body = "\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"x\"}}\n\n";
        let out = trim_images(body);
        // No double newlines introduced.
        assert!(!out.contains("\n\n"));
    }

    #[test]
    fn base64_source_without_explicit_type_still_redacts() {
        // Some CC versions emit `{"source":{"type":"base64",...}}`
        // without setting `type:"image"` at the block level. We catch
        // any block whose `source.type == "base64"`.
        let line = r#"{"type":"user","message":{"role":"user","content":[{"source":{"type":"base64","data":"PPPP"}}]}}"#;
        let out = trim_images(line);
        assert!(out.contains("[image redacted]"));
        assert!(!out.contains("PPPP"));
    }

    #[test]
    fn trim_file_writes_back_and_returns_restore_info() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("session.jsonl");
        let original = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"image\",\"source\":{\"type\":\"base64\",\"data\":\"BIN\"}}]}}\n";
        fs::write(&p, original).unwrap();

        let info = trim_file(&p).unwrap();
        assert_eq!(info.kind, "file");
        assert_eq!(info.original_path, p.display().to_string());
        assert_eq!(info.current_path.as_deref(), Some(p.display().to_string().as_str()));
        let backup_bytes = info.backup_bytes.clone().expect("trim must capture original bytes");
        assert_eq!(String::from_utf8(backup_bytes).unwrap(), original);

        let after = fs::read_to_string(&p).unwrap();
        assert!(after.contains("[image redacted]"));
        assert!(!after.contains("BIN"));
    }

    #[test]
    fn trim_file_restore_recovers_original_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("session.jsonl");
        let original = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"image\",\"source\":{\"type\":\"base64\",\"data\":\"QQ\"}}]}}\n";
        fs::write(&p, original).unwrap();
        let info = trim_file(&p).unwrap();

        // The harness `restore` impl writes backup_bytes back to
        // original_path. We replicate that here to verify the round-trip.
        let backup = info.backup_bytes.as_ref().unwrap();
        fs::write(&p, backup).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), original);
    }

    #[test]
    fn trim_file_missing_returns_err() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope.jsonl");
        assert!(trim_file(&p).is_err());
    }

    #[test]
    fn trim_file_rejects_non_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.jsonl");
        fs::write(&p, [0xff, 0xfe, 0xfd]).unwrap();
        assert!(trim_file(&p).is_err());
    }
}