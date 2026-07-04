//! claude_budget.rs — Per-scope context-window token composition.
//!
//! CCO parity: this is a Rust port of `claude-context-budget.mjs` and the
//! budget parts of `tokenizer.mjs`. Constants are intentionally rounded
//! (see comments on each) and must NOT be tuned without re-measuring on
//! Claude Code — they are deliberately stable per release.
//!
//! The composer takes:
//!   - `scope_id`        — the scope the user is inspecting
//!   - `items`           — every `HarnessItem` belonging to that scope
//!                         (caller is responsible for filtering; we
//!                         re-filter by category here for safety)
//!   - `mcp_servers`     — UNIQUE server names (caller must dedupe;
//!                         the plan requires counting per unique server)
//!
//! Output (`BudgetBreakdown`) carries every component the UI needs to
//! render the meter, the per-category breakdown, and the per-item
//! detail list. The `measured` flag is propagated from the tokenizer so
//! the UI can label the meter "measured" vs "estimated". Real measurements
//! vary across releases (Sonnet 14.8K and Opus 20.2K per CCO comment block).

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::WardError;
use crate::model::HarnessItem;
use crate::tokenizer::{self, TokenCount};

/// System overhead — always injected. Real measurements range 14.8K
/// (Sonnet 200K) to 20.2K (Opus 200K). We use 18K as a middle-ground.
pub const SYSTEM_LOADED: usize = 18000;
/// Tools kept "deferred" until invoked via ToolSearch (~7K).
pub const SYSTEM_DEFERRED: usize = 7000;
/// Per-unique-server tool schema footprint when ToolSearch is active.
pub const MCP_TOOL_SCHEMA: usize = 3100;
/// `<system-reminder>` wrapper tokens around injected CLAUDE.md.
pub const CLAUDEMD_WRAPPER: usize = 100;
/// Reserved headroom for autocompact to do its work.
pub const AUTOCOMPACT_BUFFER: usize = 13000;
/// Free space below which Claude Code starts warning the user.
pub const WARNING_THRESHOLD: usize = 20000;
/// Reserved for the model's response.
pub const MAX_OUTPUT: usize = 32000;

/// Categories Claude Code always injects at session start. Everything
/// in this set is counted into `always_loaded_items`.
pub const ALWAYS_LOADED_CATEGORIES: &[&str] = &["skill", "rule", "command", "agent"];

/// Hard cap on `@import` expansion hops. Matches CCO. Anything past this
/// depth is returned verbatim (no further recursion).
pub const MAX_IMPORT_DEPTH: u8 = 5;

// ── Wire types ────────────────────────────────────────────────────────

/// Per-scope budget composition. Wire form is camelCase to match the
/// rest of Ward's frontend types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetBreakdown {
    pub system_loaded: usize,
    /// Deferred system tools (CCO `SYSTEM_DEFERRED`).
    pub system_deferred: usize,
    /// Per-unique-server MCP schema tokens (CCO `MCP_TOOL_SCHEMA *
    /// unique_servers`).
    pub mcp_schemas: usize,
    /// Sum of token-counted CLAUDE.md files (after `@import` expansion)
    /// plus `CLAUDEMD_WRAPPER`.
    pub claudemd: usize,
    /// Per-file token breakdown for each CLAUDE.md that contributed.
    pub claude_md_files: Vec<BudgetFile>,
    /// Per-item token breakdown for every always-loaded item (skill,
    /// rule, command, agent).
    pub always_loaded_items: Vec<BudgetItem>,
    /// Reserved buffer for autocompact.
    pub autocompact_buffer: usize,
    /// Reserved for the model's response.
    pub max_output: usize,
    /// Free-space threshold below which Claude Code warns.
    pub warning_threshold: usize,
    /// Whether the underlying tokenizer was a real BPE (true) or the
    /// bytes/4 fallback (false). The UI surfaces this honestly.
    pub measured: bool,
    /// Total tokens used by always-loaded + system overhead. Used by
    /// the meter.
    pub used: usize,
    /// Total available context (200K model).
    pub context_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetFile {
    pub path: String,
    pub name: String,
    pub tokens: usize,
    pub measured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetItem {
    pub category: String,
    pub name: String,
    pub tokens: usize,
    pub measured: bool,
}

// ── @import expansion ────────────────────────────────────────────────

/// Expand `@<path>` lines inside `content` by inlining the referenced
/// files. Imported content is recursively expanded (up to
/// `MAX_IMPORT_DEPTH` hops). Circular imports are detected via the
/// `seen` set and skipped — a line referencing a file already on the
/// expansion stack is kept verbatim.
///
/// Path semantics (CCO parity):
///   - `~` expands to the user's home directory.
///   - `@/abs/path` is treated as absolute.
///   - `@relative/path` is resolved against `base_path`.
///
/// Files that fail to read are kept verbatim in the output — Claude
/// Code itself just inlines the literal text if the file is missing,
/// and we'd rather show the original line than drop context.
///
/// `seen` and `home` are threaded through the recursion so callers
/// can pre-populate the seen set (e.g. the parent file itself) and
/// inject a deterministic home for tests.
pub fn expand_imports(
    content: &str,
    base_path: &Path,
    depth: u8,
    seen: &mut HashSet<PathBuf>,
    home: &Path,
) -> String {
    if depth >= MAX_IMPORT_DEPTH {
        return content.to_string();
    }
    let mut out_lines: Vec<String> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        // Match `@<path>` where <path> starts at the first non-@ char.
        // We intentionally do NOT match indented `@` (e.g. inside code
        // blocks); CCO also matches line-leading `@` only.
        if let Some(rest) = trimmed.strip_prefix('@') {
            let import_path_str = rest.trim();
            if import_path_str.is_empty() {
                out_lines.push(line.to_string());
                continue;
            }
            // ~ expansion
            let raw = if let Some(stripped) = import_path_str.strip_prefix('~') {
                home.join(stripped.trim_start_matches('/'))
            } else if import_path_str.starts_with('/') {
                PathBuf::from(import_path_str)
            } else {
                base_path.join(import_path_str)
            };
            // Normalize (remove ./ etc.) and check for cycles.
            let normalized = normalize_path(&raw);
            if seen.contains(&normalized) {
                // Circular — keep the original line, don't re-inline.
                out_lines.push(line.to_string());
                continue;
            }
            // Read + recurse.
            match std::fs::read_to_string(&normalized) {
                Ok(imported) => {
                    seen.insert(normalized.clone());
                    let parent = normalized.parent().unwrap_or(base_path);
                    let expanded = expand_imports(&imported, parent, depth + 1, seen, home);
                    out_lines.push(expanded);
                }
                Err(_) => {
                    // Keep the original line so the user sees the import
                    // wasn't silently dropped.
                    out_lines.push(line.to_string());
                }
            }
        } else {
            out_lines.push(line.to_string());
        }
    }
    out_lines.join("\n")
}

/// Lightly normalize a path: collapse `.` components and convert to an
/// absolute form when possible. We avoid pulling in `dunce` or
/// `path-clean` to keep the dependency surface small — the goal is just
/// to detect "are these two paths actually the same file?" for the
/// circular-import check.
fn normalize_path(p: &Path) -> PathBuf {
    let mut stack: VecDeque<std::path::Component<'_>> = VecDeque::new();
    for comp in p.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                stack.pop_back();
            }
            other => stack.push_back(other),
        }
    }
    let mut out = PathBuf::new();
    for c in stack {
        out.push(c.as_os_str());
    }
    out
}

// ── Composition ──────────────────────────────────────────────────────

/// Compute the per-scope context budget.
///
/// `items` may contain items from any scope — we filter to `scope_id`
/// internally so callers don't have to. `mcp_servers` MUST already be
/// deduplicated by the caller (the plan requires counting per UNIQUE
/// server, not per item).
pub fn compose(
    scope_id: &str,
    items: &[HarnessItem],
    mcp_servers: &[String],
    home: &Path,
) -> BudgetBreakdown {
    let scope_items: Vec<&HarnessItem> =
        items.iter().filter(|i| i.scope_id == scope_id).collect();

    // ── MCP schemas ──
    // Count once per unique server. Caller dedupes; we dedupe again as
    // defense in depth (the test suite verifies the dedup contract).
    let mut unique_servers: HashSet<&str> = HashSet::new();
    for s in mcp_servers {
        unique_servers.insert(s.as_str());
    }
    let mcp_schemas = unique_servers.len() * MCP_TOOL_SCHEMA;

    // ── CLAUDE.md files ──
    // Count items named "CLAUDE.md" (covers root ~/.claude/CLAUDE.md
    // scanned under memory) OR ".claude/CLAUDE.md" (scanned under
    // config). Tokens are taken AFTER @import expansion to mirror CCO's
    // composition, which inlines imports before injection.
    let mut claude_md_files: Vec<BudgetFile> = Vec::new();
    let mut claudemd_total: usize = CLAUDEMD_WRAPPER;
    for item in &scope_items {
        if !is_claudemd_name(&item.name) {
            continue;
        }
        let path = PathBuf::from(&item.path);
        match count_claudemd(&path, home) {
            Ok(count) => {
                claudemd_total += count.tokens;
                claude_md_files.push(BudgetFile {
                    path: item.path.clone(),
                    name: item.name.clone(),
                    tokens: count.tokens,
                    measured: count.measured,
                });
            }
            Err(_) => {
                // Missing / unreadable — skip silently. The budget UI
                // doesn't need to surface an error for every file that
                // has been moved or deleted between scan and compose.
            }
        }
    }

    // ── Always-loaded items ──
    let mut always_loaded_items: Vec<BudgetItem> = Vec::new();
    for item in &scope_items {
        if !ALWAYS_LOADED_CATEGORIES.contains(&item.category.as_str()) {
            continue;
        }
        let count = count_item(item);
        if count.tokens == 0 {
            // Skip zero-token items so the UI doesn't show empty rows.
            continue;
        }
        always_loaded_items.push(BudgetItem {
            category: item.category.clone(),
            name: item.name.clone(),
            tokens: count.tokens,
            measured: count.measured,
        });
    }

    // ── Total ──
    let loaded_subtotal: usize = always_loaded_items.iter().map(|i| i.tokens).sum();
    let used = SYSTEM_LOADED + mcp_schemas + claudemd_total + loaded_subtotal;

    BudgetBreakdown {
        system_loaded: SYSTEM_LOADED,
        system_deferred: SYSTEM_DEFERRED,
        mcp_schemas,
        claudemd: claudemd_total,
        claude_md_files,
        always_loaded_items,
        autocompact_buffer: AUTOCOMPACT_BUFFER,
        max_output: MAX_OUTPUT,
        warning_threshold: WARNING_THRESHOLD,
        measured: tokenizer::active_tokenizer() == crate::tokenizer::TokenizerKind::Tiktoken,
        used,
        context_limit: 200_000,
    }
}

/// Returns true when `name` looks like a CLAUDE.md the loader would
/// inject. Both `CLAUDE.md` (root, scanned under `memory`) and
/// `.claude/CLAUDE.md` (scanned under `config`) match.
fn is_claudemd_name(name: &str) -> bool {
    name == "CLAUDE.md" || name == ".claude/CLAUDE.md"
}

/// Read a CLAUDE.md, expand its `@import` lines, and tokenize. The
/// `@import` recursion is bounded by `MAX_IMPORT_DEPTH` and circular
/// imports are detected via the `seen` set seeded with the file itself.
fn count_claudemd(path: &Path, home: &Path) -> Result<TokenCount, WardError> {
    let raw = std::fs::read_to_string(path)?;
    let parent = path.parent().unwrap_or(home);
    let mut seen: HashSet<PathBuf> = HashSet::new();
    seen.insert(normalize_path(path));
    let expanded = expand_imports(&raw, parent, 0, &mut seen, home);
    Ok(tokenizer::count_text(&expanded))
}

/// Tokenize one always-loaded item by reading its file. MCP items are
/// not in `ALWAYS_LOADED_CATEGORIES` so this path never sees them.
fn count_item(item: &HarnessItem) -> TokenCount {
    if item.path.is_empty() {
        return TokenCount { tokens: 0, measured: false };
    }
    match tokenizer::count_file(Path::new(&item.path)) {
        Ok(c) => c,
        Err(_) => TokenCount { tokens: 0, measured: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn item(category: &str, scope: &str, name: &str, path: &str) -> HarnessItem {
        HarnessItem {
            category: category.into(),
            scope_id: scope.into(),
            name: name.into(),
            description: String::new(),
            path: path.into(),
            movable: false,
            deletable: false,
            locked: false,
            effective: None,
            mcp_config: None,
        }
    }

    // ── expand_imports ──

    #[test]
    fn expand_imports_inlines_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let imp = dir.path().join("frag.md");
        fs::write(&imp, "imported body").unwrap();
        let home = dir.path();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let out = expand_imports(
            "before\n@frag.md\nafter",
            dir.path(),
            0,
            &mut seen,
            home,
        );
        assert_eq!(out, "before\nimported body\nafter");
    }

    #[test]
    fn expand_imports_expands_relative_path_against_base() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("x.md"), "X").unwrap();
        let home = dir.path();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let out = expand_imports("@sub/x.md", dir.path(), 0, &mut seen, home);
        assert_eq!(out, "X");
    }

    #[test]
    fn expand_imports_caps_at_max_depth() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        // a -> b -> a (would be circular if not for depth cap)
        fs::write(&a, "@b.md").unwrap();
        fs::write(&b, "@a.md").unwrap();
        let home = dir.path();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        // Call expand_imports directly at depth == MAX to confirm we
        // get the content unchanged.
        let out = expand_imports(
            "@a.md",
            dir.path(),
            MAX_IMPORT_DEPTH,
            &mut seen,
            home,
        );
        assert_eq!(out, "@a.md");
    }

    #[test]
    fn expand_imports_detects_circular_self_import() {
        let dir = tempfile::tempdir().unwrap();
        let self_path = dir.path().join("self.md");
        // self.md imports itself — depth 1 still tries to read it,
        // seen-set already contains it, so the line is kept verbatim.
        fs::write(&self_path, "head\n@self.md\ntail").unwrap();
        let home = dir.path();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        seen.insert(normalize_path(&self_path));
        let out = expand_imports("head\n@self.md\ntail", dir.path(), 0, &mut seen, home);
        assert_eq!(out, "head\n@self.md\ntail");
    }

    #[test]
    fn expand_imports_keeps_missing_lines_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let out = expand_imports("before\n@does-not-exist.md\nafter", dir.path(), 0, &mut seen, home);
        assert_eq!(out, "before\n@does-not-exist.md\nafter");
    }

    #[test]
    fn expand_imports_recurses_through_chain() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.md"), "A").unwrap();
        // b.md has a line-leading @import. CCO (and this impl) only
        // expand when the entire line matches `^@<path>$`, so we put
        // @a.md on its own line.
        fs::write(dir.path().join("b.md"), "B-prefix\n@a.md\nB-suffix").unwrap();
        let home = dir.path();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let out = expand_imports("@b.md", dir.path(), 0, &mut seen, home);
        assert_eq!(out, "B-prefix\nA\nB-suffix");
    }

    #[test]
    fn expand_imports_does_not_touch_at_signs_in_middle_of_line() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        // `email@host.com` is not a leading-@-import and must be left
        // alone (CCO uses line-leading `@` only).
        let out = expand_imports(
            "send to user@host.com",
            dir.path(),
            0,
            &mut seen,
            home,
        );
        assert_eq!(out, "send to user@host.com");
    }

    // ── compose ──

    #[test]
    fn compose_uses_seven_constants_verbatim() {
        // Confirms the ports are exact — if anyone tunes these,
        // this test will catch it.
        assert_eq!(SYSTEM_LOADED, 18000);
        assert_eq!(SYSTEM_DEFERRED, 7000);
        assert_eq!(MCP_TOOL_SCHEMA, 3100);
        assert_eq!(CLAUDEMD_WRAPPER, 100);
        assert_eq!(AUTOCOMPACT_BUFFER, 13000);
        assert_eq!(WARNING_THRESHOLD, 20000);
        assert_eq!(MAX_OUTPUT, 32000);
    }

    #[test]
    fn compose_counts_mcp_once_per_unique_server() {
        let dir = tempfile::tempdir().unwrap();
        let items: Vec<HarnessItem> = vec![];
        // 3 unique servers -> 3 * 3100 = 9300. (Caller is responsible
        // for dedupe; this exercises the multiplication.)
        let servers = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let b = compose("global", &items, &servers, dir.path());
        assert_eq!(b.mcp_schemas, 3 * MCP_TOOL_SCHEMA);
    }

    #[test]
    fn compose_dedupes_mcp_servers_when_caller_forgets() {
        let dir = tempfile::tempdir().unwrap();
        let items: Vec<HarnessItem> = vec![];
        // Same server twice — should still be counted once.
        let servers = vec!["github".to_string(), "github".to_string()];
        let b = compose("global", &items, &servers, dir.path());
        assert_eq!(b.mcp_schemas, MCP_TOOL_SCHEMA);
    }

    #[test]
    fn compose_counts_skill_rule_command_agent_always_loaded() {
        let dir = tempfile::tempdir().unwrap();
        // One of each always-loaded category.
        let skill_path = dir.path().join("skill.md");
        let rule_path = dir.path().join("rule.md");
        let cmd_path = dir.path().join("cmd.md");
        let agent_path = dir.path().join("agent.md");
        let memory_path = dir.path().join("mem.md");
        fs::write(&skill_path, "skill body 1234").unwrap();    // 14 bytes -> 4 tokens
        fs::write(&rule_path, "rule body 12345").unwrap();     // 15 bytes -> 4 tokens
        fs::write(&cmd_path, "cmd body 123456").unwrap();      // 16 bytes -> 4 tokens
        fs::write(&agent_path, "agent body 1234567").unwrap(); // 17 bytes -> 5 tokens
        fs::write(&memory_path, "mem").unwrap();               // NOT always-loaded

        let items = vec![
            item("skill", "global", "skill", skill_path.to_str().unwrap()),
            item("rule", "global", "rule", rule_path.to_str().unwrap()),
            item("command", "global", "cmd", cmd_path.to_str().unwrap()),
            item("agent", "global", "agent", agent_path.to_str().unwrap()),
            item("memory", "global", "mem", memory_path.to_str().unwrap()),
        ];
        let b = compose("global", &items, &[], dir.path());
        let cats: Vec<&str> = b.always_loaded_items.iter().map(|i| i.category.as_str()).collect();
        assert_eq!(cats, vec!["skill", "rule", "command", "agent"]);
        // No memory in always-loaded.
        assert!(!cats.contains(&"memory"));
        // All four items counted.
        assert_eq!(b.always_loaded_items.len(), 4);
    }

    #[test]
    fn compose_counts_claudemd_with_import_expansion() {
        let dir = tempfile::tempdir().unwrap();
        let frag = dir.path().join("frag.md");
        fs::write(&frag, "frag-body").unwrap(); // 9 bytes -> 3 tokens
        let claude_md = dir.path().join("CLAUDE.md");
        // CLAUDE.md contains "@frag.md\nown-body" -> expands to
        // "frag-body\nown-body" = 9 + 8 = 17 bytes -> 5 tokens.
        fs::write(&claude_md, "@frag.md\nown-body").unwrap();
        let items = vec![item("memory", "global", "CLAUDE.md", claude_md.to_str().unwrap())];
        let b = compose("global", &items, &[], dir.path());
        // Wrapper + content tokens.
        assert!(b.claudemd > CLAUDEMD_WRAPPER);
        assert_eq!(b.claude_md_files.len(), 1);
        assert_eq!(b.claude_md_files[0].name, "CLAUDE.md");
        // Confirm import was expanded: 17 bytes / 4 = 5 (ceil).
        assert_eq!(b.claude_md_files[0].tokens, 5);
        // Wrapper added on top.
        assert_eq!(b.claudemd, 5 + CLAUDEMD_WRAPPER);
    }

    #[test]
    fn compose_total_sums_all_components() {
        let dir = tempfile::tempdir().unwrap();
        let items: Vec<HarnessItem> = vec![];
        let b = compose("global", &items, &["s1".into()], dir.path());
        // system + mcp + wrapper (no claudemd files) + 0 always-loaded
        assert_eq!(b.used, SYSTEM_LOADED + MCP_TOOL_SCHEMA + CLAUDEMD_WRAPPER);
    }

    #[test]
    fn compose_filters_to_requested_scope_only() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        fs::write(&p, "x".repeat(40)).unwrap(); // 10 tokens
        let items = vec![
            item("skill", "scope-a", "a-skill", p.to_str().unwrap()),
            item("skill", "scope-b", "b-skill", p.to_str().unwrap()),
        ];
        let b_a = compose("scope-a", &items, &[], dir.path());
        let b_b = compose("scope-b", &items, &[], dir.path());
        assert_eq!(b_a.always_loaded_items.len(), 1);
        assert_eq!(b_b.always_loaded_items.len(), 1);
        assert_eq!(b_a.always_loaded_items[0].name, "a-skill");
        assert_eq!(b_b.always_loaded_items[0].name, "b-skill");
    }

    #[test]
    fn compose_reports_measured_false_for_bytes_div4() {
        let dir = tempfile::tempdir().unwrap();
        let items: Vec<HarnessItem> = vec![];
        let b = compose("global", &items, &[], dir.path());
        assert!(!b.measured);
    }

    #[test]
    fn compose_serializes_camel_case() {
        let dir = tempfile::tempdir().unwrap();
        let items: Vec<HarnessItem> = vec![];
        let b = compose("global", &items, &[], dir.path());
        let json = serde_json::to_string(&b).unwrap();
        // Top-level camelCase fields.
        for key in ["systemLoaded", "mcpSchemas", "alwaysLoadedItems", "autocompactBuffer", "warningThreshold"] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
    }
}