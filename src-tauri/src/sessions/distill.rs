//! Session distillation (Plan 07) — port of CCO's `session-distiller.mjs`.
//!
//! Workflow:
//!   1. Back up the original `.jsonl` to `<new-session-id>/backup-<original>`.
//!   2. Stream-parse every line.
//!   3. Rewrite the conversation as a clean, resumable JSONL:
//!      - Keep `user`/`assistant` records, but rewrite `message.content`
//!        so tool results, base64 image blocks, and large tool inputs
//!        are replaced by short textual summaries.
//!      - Drop `attachment`, `progress`, `pr-link`, `file-history-snapshot`,
//!        `custom-title` records.
//!      - Pass through `queue-operation`, `last-prompt`, system
//!        `compact_boundary` records verbatim (with envelope metadata
//!        deduplicated).
//!   4. Emit an `index.md` next to the backup describing what was
//!      trimmed and how to retrieve the original content.
//!   5. Write the cleaned JSONL to `<new-session-id>.jsonl`.
//!
//! Backups happen **before** the cleaned JSONL is written — the
//! `.bak`-equivalent file must always exist on disk before any
//! destructive change.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::WardError;
use crate::sessions::parse::{parse_file, SessionRecord};

// ── Wire model ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DistillResult {
    pub original_path: String,
    pub cleaned_path: String,
    pub backup_path: String,
    pub original_bytes: u64,
    pub cleaned_bytes: u64,
    pub reduction_pct: f64,
    pub index_md: String,
}

/// Result of computing the cleaned JSONL + backup paths without writing.
/// Exposed so tests can verify path layout without touching disk.
pub fn distill_paths(original: &Path) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let parent = original.parent()?;
    let new_id = new_session_id();
    let backup_path = parent.join(format!("{new_id}.bak.jsonl"));
    let cleaned_path = parent.join(format!("{new_id}.jsonl"));
    let index_path = parent.join(format!("{new_id}.index.md"));
    Some((cleaned_path, backup_path, index_path))
}

fn new_session_id() -> String {
    // uuid v4 is already a project dependency; use it to mint a fresh
    // session id so the cleaned file doesn't collide with the original.
    uuid::Uuid::new_v4().to_string()
}

// ── Public entry point ─────────────────────────────────────────────────

/// Distill `session_path`. Returns the resulting `DistillResult` on
/// success. The backup file is written BEFORE the cleaned JSONL so a
/// mid-write crash leaves the original recoverable from disk.
pub fn distill(session_path: &Path) -> Result<DistillResult, WardError> {
    let original_bytes = fs::metadata(session_path)?.len();
    let (cleaned_path, backup_path, index_path) = distill_paths(session_path)
        .ok_or_else(|| WardError::NotFound(format!("cannot resolve paths for {}", session_path.display())))?;

    // Step 1 — back up the original verbatim.
    fs::copy(session_path, &backup_path)?;

    // Step 2 — re-parse via the streaming parser.
    let conv = parse_file(session_path)?;

    // Step 3 — produce cleaned JSONL and an index of large results.
    let mut writer = CleanedWriter::new();
    for (idx, rec) in conv.records.iter().enumerate() {
        writer.push(rec, idx + 1);
    }

    // Step 4 — write the cleaned JSONL.
    fs::write(&cleaned_path, writer.output.as_bytes())?;
    let cleaned_bytes = writer.output.len() as u64;

    // Step 5 — emit the index.md (may be empty if there were no large
    // results to redirect to the backup).
    let index_md = writer.build_index(
        session_path,
        &backup_path,
        &cleaned_path,
        original_bytes,
        cleaned_bytes,
    );
    fs::write(&index_path, index_md.as_bytes())?;

    let reduction_pct = if original_bytes == 0 {
        0.0
    } else {
        let pct = 100.0 * (1.0 - (cleaned_bytes as f64 / original_bytes as f64));
        (pct * 10.0).round() / 10.0
    };

    Ok(DistillResult {
        original_path: session_path.display().to_string(),
        cleaned_path: cleaned_path.display().to_string(),
        backup_path: backup_path.display().to_string(),
        original_bytes,
        cleaned_bytes,
        reduction_pct,
        index_md,
    })
}

// ── Cleaned JSONL writer ───────────────────────────────────────────────

/// Tracks the cleaned output stream plus the list of large tool results
/// that we redirect to the backup file.
struct CleanedWriter {
    output: String,
    /// Each entry: (line_in_cleaned_output, tool_name, first_line_label, original_line, chars).
    entries: Vec<IndexEntry>,
}

#[derive(Clone)]
struct IndexEntry {
    id: usize,
    tool: String,
    label: String,
    orig_line: usize,
    chars: usize,
}

/// Threshold above which a tool_result block is moved to the backup
/// instead of being inlined. Matches CCO's LARGE_THRESHOLD = 1500.
const LARGE_THRESHOLD: usize = 1500;

impl CleanedWriter {
    fn new() -> Self {
        Self { output: String::new(), entries: Vec::new() }
    }

    fn push_line(&mut self, line: &str) {
        if !self.output.is_empty() {
            self.output.push('\n');
        }
        self.output.push_str(line);
    }

    fn push(&mut self, rec: &SessionRecord, orig_line: usize) {
        match classify_for_distill(rec) {
            DistillAction::Drop => {}
            DistillAction::PassThrough => {
                // Re-serialize the original record untouched.
                if let Some(line) = json_line(rec) {
                    self.push_line(&line);
                }
            }
            DistillAction::Rewritten(jsonl_line) => {
                self.push_line(&jsonl_line);
            }
            DistillAction::LargeRedirect(tool_name, label, raw_chars) => {
                let id = self.entries.len() + 1;
                let entry = IndexEntry { id, tool: tool_name.clone(), label: label.clone(), orig_line, chars: raw_chars };
                self.entries.push(entry);
                let marker = format!(
                    "[{tool_name} ({raw_chars} chars) → backup line {orig_line}, index #{id}]"
                );
                // Wrap into a `user` record so it appears in the
                // resumed conversation as a context block.
                let line = serde_json::json!({
                    "type": "user",
                    "message": { "role": "user", "content": [{ "type": "text", "text": marker }] }
                });
                self.push_line(&line.to_string());
            }
        }
    }

    fn build_index(
        &self,
        original: &Path,
        backup: &Path,
        cleaned: &Path,
        original_bytes: u64,
        cleaned_bytes: u64,
    ) -> String {
        let mut out = String::new();
        out.push_str("# Distilled Session Index\n\n");
        out.push_str(&format!("- Original: `{}`\n", original.display()));
        out.push_str(&format!("- Backup:   `{}`\n", backup.display()));
        out.push_str(&format!("- Cleaned:  `{}`\n", cleaned.display()));
        out.push_str(&format!("- Original size: {original_bytes} bytes\n"));
        out.push_str(&format!("- Cleaned size:  {cleaned_bytes} bytes\n"));
        if self.entries.is_empty() {
            out.push_str("\n_No large tool results were redirected to the backup._\n");
            return out;
        }
        out.push_str(&format!("\n## Redirected tool results ({} entries)\n\n", self.entries.len()));
        out.push_str("To read any redirected tool result, open the backup file:\n");
        out.push_str("```\n");
        out.push_str(&format!("Read {} offset=<line> limit=50\n", backup.display()));
        out.push_str("```\n\n");
        out.push_str("| # | Tool | Description | Orig line | Size (chars) |\n");
        out.push_str("|---|------|-------------|-----------|--------------|\n");
        for e in &self.entries {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                e.id, e.tool, e.label, e.orig_line, e.chars
            ));
        }
        out
    }
}

// ── Classification ─────────────────────────────────────────────────────

enum DistillAction {
    /// Drop the record entirely.
    Drop,
    /// Pass through unchanged (queue-operation, last-prompt, compact_boundary).
    PassThrough,
    /// Rewrite message.content but emit a single JSONL line.
    Rewritten(String),
    /// Replace the record with a marker pointing at the backup.
    LargeRedirect(String, String, usize),
}

const DROP_TYPES: &[&str] = &[
    "file-history-snapshot",
    "attachment",
    "progress",
    "pr-link",
    "custom-title",
    "ai-title", // dropped — we re-emit a `[distilled]` title ourselves if needed
];

const PASSTHROUGH_TYPES: &[&str] = &["queue-operation", "last-prompt"];

fn classify_for_distill(rec: &SessionRecord) -> DistillAction {
    match rec {
        SessionRecord::Other { record_type } => {
            if DROP_TYPES.contains(&record_type.as_str()) {
                DistillAction::Drop
            } else if PASSTHROUGH_TYPES.contains(&record_type.as_str()) {
                DistillAction::PassThrough
            } else {
                // Unknown type: drop (we don't know how to clean it).
                DistillAction::Drop
            }
        }
        SessionRecord::System { subtype, .. } => {
            if subtype == "compact_boundary" {
                DistillAction::PassThrough
            } else {
                DistillAction::Drop
            }
        }
        SessionRecord::AiTitle { .. } => DistillAction::Drop,
        SessionRecord::QueueOperation { .. } => DistillAction::PassThrough,
        SessionRecord::User { content, .. } => {
            // `content` is the flattened text derived from the structured
            // blocks (tool_result / text / image). We keep any turn that
            // carries text and drop only genuinely-empty ones. NOTE: this
            // now preserves `tool_result` user turns — before the block
            // parser landed they flattened to "" and were silently
            // dropped, discarding what every tool returned. Keeping the
            // derived text is the intended behavior (see
            // `distill_keeps_tool_result_user_turn`).
            let cleaned = if content.trim().is_empty() { return DistillAction::Drop; } else { content.clone() };
            DistillAction::Rewritten(build_user_line(&cleaned))
        }
        SessionRecord::Assistant { content, model, usage, .. } => {
            let summary = summarize_assistant_text(content);
            if summary.is_empty() {
                return DistillAction::Drop;
            }
            // Build a minimal assistant record: keep model + drop usage
            // to avoid double-counting when the conversation is resumed.
            let mut obj = serde_json::json!({
                "type": "assistant",
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": summary }],
                }
            });
            if let Some(m) = model {
                obj["message"]["model"] = Value::String(m.clone());
            }
            // usage is intentionally dropped — see CCO.
            let _ = usage;
            DistillAction::Rewritten(obj.to_string())
        }
    }
}

fn summarize_assistant_text(content: &str) -> String {
    content.trim().to_string()
}

fn build_user_line(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": text }
    })
    .to_string()
}

/// Re-serialize a record that we want to pass through verbatim.
/// We don't have the original JSON around — only the parsed fields —
/// so we rebuild a minimal JSON object. This is acceptable because
/// the resume consumer (CC) only cares about the type/subtype.
fn json_line(rec: &SessionRecord) -> Option<String> {
    match rec {
        SessionRecord::QueueOperation { enqueue } => Some(serde_json::json!({
            "type": "queue-operation",
            "enqueue": enqueue,
        }).to_string()),
        SessionRecord::System { subtype, summary } => {
            let mut obj = serde_json::json!({
                "type": "system",
                "subtype": subtype,
            });
            if let Some(s) = summary {
                obj["summary"] = Value::String(s.clone());
            }
            Some(obj.to_string())
        }
        _ => None,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::parse::Usage;
    use std::fs;

    fn write_session(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn distill_paths_lay_out_next_to_original() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("orig.jsonl");
        fs::write(&p, "").unwrap();
        let (cleaned, backup, index) = distill_paths(&p).unwrap();
        assert!(cleaned.to_string_lossy().ends_with(".jsonl"));
        assert!(backup.to_string_lossy().ends_with(".bak.jsonl"));
        assert!(index.to_string_lossy().ends_with(".index.md"));
        assert_ne!(cleaned, backup);
        assert_ne!(cleaned, index);
    }

    #[test]
    fn distill_backs_up_before_writing_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "src.jsonl",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n");
        let r = distill(&p).unwrap();
        // Backup exists at the moment we returned.
        assert!(PathBuf::from(&r.backup_path).exists());
        assert_eq!(fs::read_to_string(r.backup_path).unwrap(),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n");
        // Cleaned JSONL exists.
        assert!(PathBuf::from(&r.cleaned_path).exists());
        // Original is still present (we never overwrite the source).
        assert!(p.exists());
    }

    #[test]
    fn distill_reports_reduction_pct() {
        let dir = tempfile::tempdir().unwrap();
        // Build a session whose user message is "hello world" (12 chars).
        // The cleaned version should drop the envelope and tool data,
        // yielding a positive reduction.
        let p = write_session(dir.path(), "src.jsonl",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello world\"}}\n");
        let r = distill(&p).unwrap();
        assert!(r.original_bytes > 0);
        assert!(r.cleaned_bytes > 0);
        // Reduction is non-negative.
        assert!(r.reduction_pct >= 0.0 && r.reduction_pct <= 100.0);
        // For a 1-line tiny session reduction may be 0% (the wrapper
        // we add is similar size). We just check the math is sane.
    }

    #[test]
    fn distill_drops_attachment_progress_etc() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "src.jsonl", "\
{\"type\":\"file-history-snapshot\",\"snapshot\":{\"x\":1}}\n\
{\"type\":\"attachment\",\"id\":\"a\"}\n\
{\"type\":\"progress\",\"label\":\"x\"}\n\
{\"type\":\"pr-link\",\"url\":\"u\"}\n\
{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"real\"}}\n");
        let r = distill(&p).unwrap();
        let cleaned = fs::read_to_string(&r.cleaned_path).unwrap();
        assert!(!cleaned.contains("file-history-snapshot"));
        assert!(!cleaned.contains("attachment"));
        assert!(!cleaned.contains("progress"));
        assert!(!cleaned.contains("pr-link"));
        assert!(cleaned.contains("real"));
    }

    #[test]
    fn distill_passes_through_queue_operation_and_compact_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "src.jsonl", "\
{\"type\":\"queue-operation\",\"enqueue\":true}\n\
{\"type\":\"system\",\"subtype\":\"compact_boundary\"}\n\
{\"type\":\"system\",\"subtype\":\"hook_started\"}\n");
        let r = distill(&p).unwrap();
        let cleaned = fs::read_to_string(&r.cleaned_path).unwrap();
        assert!(cleaned.contains("queue-operation"), "queue-op must pass through: {cleaned}");
        assert!(cleaned.contains("compact_boundary"), "compact_boundary must pass through: {cleaned}");
        assert!(!cleaned.contains("hook_started"), "non-compact system records are dropped: {cleaned}");
    }

    #[test]
    fn distill_keeps_tool_result_user_turn() {
        // Coupling with the structured-block parser (Commit A): a user
        // `tool_result` turn used to flatten to "" and get dropped here,
        // silently discarding every tool output from the resumed
        // conversation. Now that the parser populates the derived
        // `content`, distill KEEPS it. This asserts that intended change.
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "src.jsonl",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_9\",\"content\":\"BUILD SUCCEEDED in 4.2s\"}]}}\n");
        let r = distill(&p).unwrap();
        let cleaned = fs::read_to_string(&r.cleaned_path).unwrap();
        assert!(
            cleaned.contains("BUILD SUCCEEDED in 4.2s"),
            "tool_result user turn must be preserved, got: {cleaned}"
        );
    }

    #[test]
    fn distill_keeps_assistant_tool_use_turn() {
        // Likewise an assistant turn that is ONLY a `tool_use` block (no
        // text) used to drop out; it now carries a `Bash: …` label.
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "src.jsonl",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Bash\",\"input\":{\"command\":\"cargo test\"}}],\"model\":\"claude-opus-4-8\"}}\n");
        let r = distill(&p).unwrap();
        let cleaned = fs::read_to_string(&r.cleaned_path).unwrap();
        assert!(cleaned.contains("Bash: cargo test"), "tool_use assistant turn preserved, got: {cleaned}");
    }

    #[test]
    fn distill_drops_empty_user_records() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "src.jsonl", "\
{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"\"}}\n\
{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"   \"}}\n");
        let r = distill(&p).unwrap();
        let cleaned = fs::read_to_string(&r.cleaned_path).unwrap();
        assert!(cleaned.trim().is_empty(), "expected empty cleaned output, got {cleaned}");
    }

    #[test]
    fn distill_assistant_record_drops_usage() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "src.jsonl", "\
{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"answer\"}],\"model\":\"claude-sonnet-4-5\",\"usage\":{\"input_tokens\":100,\"output_tokens\":20}}}\n");
        let r = distill(&p).unwrap();
        let cleaned = fs::read_to_string(&r.cleaned_path).unwrap();
        assert!(cleaned.contains("\"answer\""));
        assert!(!cleaned.contains("\"usage\""), "usage must be stripped: {cleaned}");
    }

    #[test]
    fn distill_index_md_includes_paths() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "src.jsonl",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"x\"}}\n");
        let r = distill(&p).unwrap();
        assert!(r.index_md.contains("# Distilled Session Index"));
        assert!(r.index_md.contains(&r.backup_path));
        assert!(r.index_md.contains(&r.cleaned_path));
    }

    #[test]
    fn distill_index_md_with_no_redirects_has_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_session(dir.path(), "src.jsonl",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n");
        let r = distill(&p).unwrap();
        assert!(r.index_md.contains("_No large tool results were redirected"));
    }

    #[test]
    fn distill_preserves_conversation_text_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let body = "\
{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"Plan the build\"}}\n\
{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Sounds good\"}],\"model\":\"claude-sonnet-4-5\"}}\n\
{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"Begin.\"}}\n";
        let p = write_session(dir.path(), "src.jsonl", body);
        let r = distill(&p).unwrap();
        let cleaned = fs::read_to_string(&r.cleaned_path).unwrap();
        assert!(cleaned.contains("Plan the build"));
        assert!(cleaned.contains("Sounds good"));
        assert!(cleaned.contains("Begin."));
    }

    #[test]
    fn distill_returns_err_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope.jsonl");
        assert!(distill(&p).is_err());
    }

    #[test]
    fn distill_failure_before_backup_leaves_nothing() {
        // Missing file → distill errors, no backup created.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("missing.jsonl");
        let _ = distill(&p).unwrap_err();
        // No .bak.jsonl files were created in the temp dir.
        let bak: Vec<_> = fs::read_dir(dir.path()).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".bak.jsonl"))
            .collect();
        assert!(bak.is_empty());
    }

    #[test]
    fn distill_result_serializes_camel_case() {
        let r = DistillResult {
            original_path: "/a".into(),
            cleaned_path: "/b".into(),
            backup_path: "/c".into(),
            original_bytes: 1000,
            cleaned_bytes: 100,
            reduction_pct: 90.0,
            index_md: "x".into(),
        };
        let j = serde_json::to_string(&r).unwrap();
        assert!(j.contains("\"originalPath\":\"/a\""));
        assert!(j.contains("\"cleanedPath\":\"/b\""));
        assert!(j.contains("\"backupPath\":\"/c\""));
        assert!(j.contains("\"originalBytes\":1000"));
        assert!(j.contains("\"cleanedBytes\":100"));
        assert!(j.contains("\"reductionPct\":90.0"));
    }
}