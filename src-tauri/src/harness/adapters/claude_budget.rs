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
/// Estimated (labeled as such in the UI); do not tune without re-measuring.
pub const SYSTEM_LOADED: usize = 18000;
/// Tools kept "deferred" until invoked via ToolSearch (~7K). Estimated.
pub const SYSTEM_DEFERRED: usize = 7000;
/// Per-unique-server tool schema footprint, DEFERRED by default (Tool
/// Search only pulls a server's schemas when the model invokes one of
/// its tools). Estimated.
pub const MCP_TOOL_SCHEMA: usize = 3100;
/// Always-on MCP tool-*names* line (~120). Tool Search injects a short
/// index of available tool names even though the full schemas stay
/// deferred. Counted once when ≥1 enabled server exists. Estimated.
pub const MCP_TOOL_NAMES: usize = 120;
/// `<system-reminder>` wrapper tokens around injected CLAUDE.md.
pub const CLAUDEMD_WRAPPER: usize = 100;
/// One-time `<available_skills>` boilerplate the Skill tool injects when
/// any model-invocable skill (or custom slash-command) exists. Estimated.
pub const SKILL_BOILERPLATE: usize = 400;
/// Per-description character cap applied to each skill/command listing
/// entry before tokenizing (Claude Code truncates long descriptions).
pub const SKILL_DESC_CAP: usize = 1536;
/// Percent of the context limit the whole skill/command listing may
/// occupy. The metadata block is capped as a whole (the bodies are
/// deferred), so 100 tiny descriptions can't blow past this ceiling.
pub const SKILL_LISTING_BUDGET_PCT: usize = 1;
/// First-N-lines cap Claude Code applies when loading a `MEMORY.md`
/// index into always-on context.
pub const MEMORY_MAX_LINES: usize = 200;
/// Byte cap applied (after the line cap) to a loaded `MEMORY.md` index.
pub const MEMORY_MAX_BYTES: usize = 25_000;
/// Reserved headroom for autocompact to do its work. Estimated.
pub const AUTOCOMPACT_BUFFER: usize = 13000;
/// Free space below which Claude Code starts warning the user. Estimated.
pub const WARNING_THRESHOLD: usize = 20000;
/// Reserved for the model's response. Estimated.
pub const MAX_OUTPUT: usize = 32000;
/// Default model context window (Claude Sonnet/Opus 200K). NOT hardcoded
/// at the call sites — `compose_with_limit` accepts any limit (e.g. a
/// 1M-token model) and scales the skill-listing budget off it.
pub const DEFAULT_CONTEXT_LIMIT: usize = 200_000;

/// Hard cap on `@import` expansion hops. Claude Code follows imports up
/// to 4 hops; anything past this depth is returned verbatim.
pub const MAX_IMPORT_DEPTH: u8 = 4;

// ── Wire types ────────────────────────────────────────────────────────

/// Per-scope budget composition. Wire form is camelCase to match the
/// rest of Ward's frontend types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetBreakdown {
    pub system_loaded: usize,
    /// Text of an active non-default output style, folded into the
    /// always-on system overhead (0 when the default style is active).
    pub output_style: usize,
    /// Deferred system tools (CCO `SYSTEM_DEFERRED`).
    pub system_deferred: usize,
    /// Per-unique-enabled-server MCP schema tokens
    /// (`MCP_TOOL_SCHEMA * unique_servers`). DEFERRED — part of
    /// `deferred_total`, NOT `used`.
    pub mcp_schemas: usize,
    /// Always-on MCP tool-names line (`MCP_TOOL_NAMES` when ≥1 enabled
    /// server exists, else 0). Part of `used`.
    pub mcp_tool_names: usize,
    /// Sum of token-counted CLAUDE.md files (after `@import` expansion)
    /// plus `CLAUDEMD_WRAPPER`.
    pub claudemd: usize,
    /// Per-file token breakdown for each CLAUDE.md that contributed.
    pub claude_md_files: Vec<BudgetFile>,
    /// Always-on skill+command *listing* (`"name": description`),
    /// capped at `SKILL_LISTING_BUDGET_PCT %` of the context limit.
    pub skill_listing: usize,
    /// The uncapped listing total, so the UI can show "capped from N".
    pub skill_listing_raw: usize,
    /// `<available_skills>` boilerplate (0 when no skill/command exists).
    pub skill_boilerplate: usize,
    /// Always-on subagent *listing* (`name: description` via the Agent
    /// tool). Uncapped — agents are few.
    pub agent_listing: usize,
    /// Per-item token breakdown for full-content always-on items: rules
    /// WITHOUT `paths:` and the `MEMORY.md` index.
    pub always_loaded_items: Vec<BudgetItem>,
    /// Per-item metadata rows behind `skill_listing` / `agent_listing`
    /// (skills, commands, agents) — for the detail list. These do NOT
    /// sum into `used`; the capped scalars above do.
    pub metadata_items: Vec<BudgetItem>,
    /// On-invoke / DEFERRED per-item bodies: skill/command/agent bodies
    /// and `paths:`-scoped rules. Loaded only when invoked, so they are
    /// NOT part of `used`.
    pub deferred_items: Vec<BudgetItem>,
    /// Total deferred tokens: `system_deferred` + `mcp_schemas` +
    /// the sum of `deferred_items`. Surfaced as a separate figure.
    pub deferred_total: usize,
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
    /// Total available context (200K default, but any model limit —
    /// e.g. a 1M-token model — flows through `compose_with_limit`).
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
    // Track fenced code blocks (``` or ~~~). `@import` lines inside a
    // fence — or written as an inline code span (`` `@path` ``) — are
    // examples, not real imports, and Claude Code leaves them verbatim.
    let mut in_fence = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if is_code_fence(trimmed) {
            in_fence = !in_fence;
            out_lines.push(line.to_string());
            continue;
        }
        if in_fence || trimmed.starts_with('`') {
            // Inside a fenced block or an inline code span — keep verbatim.
            out_lines.push(line.to_string());
            continue;
        }
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

/// True when a (leading-trimmed) line opens or closes a Markdown code
/// fence — three-or-more backticks or tildes, optionally followed by an
/// info string.
fn is_code_fence(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    (bytes.starts_with(b"```") ) || (bytes.starts_with(b"~~~"))
}

/// Strip block-level HTML comments (`<!-- … -->`, possibly multi-line)
/// before tokenizing. Official docs: Claude Code strips these before
/// injecting CLAUDE.md / rules into context, so they cost no tokens.
pub fn strip_html_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end_rel) => {
                // Skip the comment including the closing `-->`.
                rest = &rest[start + end_rel + 3..];
            }
            None => {
                // Unterminated comment — drop the remainder (matches the
                // non-greedy `<!--[\s\S]*?-->` intent: nothing after an
                // open-without-close survives).
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

// ── Composition ──────────────────────────────────────────────────────

/// Compute the per-scope context budget for the default 200K model.
///
/// Thin wrapper over [`compose_with_limit`] — see it for the full
/// model. Existing call sites (and the 200K UI) use this.
pub fn compose(
    scope_id: &str,
    items: &[HarnessItem],
    mcp_servers: &[String],
    home: &Path,
) -> BudgetBreakdown {
    compose_with_limit(scope_id, items, mcp_servers, home, DEFAULT_CONTEXT_LIMIT)
}

/// Compute the per-scope context budget against an explicit
/// `context_limit` (so a 1M-token model scales the same code path —
/// the skill-listing budget is `SKILL_LISTING_BUDGET_PCT %` of *this*
/// limit, never a hardcoded 200K).
///
/// `items` may contain items from any scope — we filter to `scope_id`
/// internally so callers don't have to. `mcp_servers` MUST already be
/// deduplicated AND filtered to *enabled* servers by the caller.
pub fn compose_with_limit(
    scope_id: &str,
    items: &[HarnessItem],
    mcp_servers: &[String],
    home: &Path,
    context_limit: usize,
) -> BudgetBreakdown {
    let scope_items: Vec<&HarnessItem> =
        items.iter().filter(|i| i.scope_id == scope_id).collect();

    // ── MCP ──
    // Count once per unique enabled server. Caller dedupes + filters to
    // enabled; we dedupe again as defense in depth. Tool schemas are
    // DEFERRED (Tool Search pulls them on invoke); only a short
    // tool-*names* index is always-on.
    let mut unique_servers: HashSet<&str> = HashSet::new();
    for s in mcp_servers {
        unique_servers.insert(s.as_str());
    }
    let mcp_schemas = unique_servers.len() * MCP_TOOL_SCHEMA;
    let mcp_tool_names = if unique_servers.is_empty() { 0 } else { MCP_TOOL_NAMES };

    // ── CLAUDE.md files ──
    // Count ancestor CLAUDE.md / CLAUDE.local.md / managed items. The
    // same path can surface under both `memory` and `config` categories
    // (the scanner emits it in each) — dedupe by path so it's counted
    // ONCE. Tokens are taken AFTER @import expansion + HTML-comment
    // stripping to mirror what Claude Code injects.
    let mut claude_md_files: Vec<BudgetFile> = Vec::new();
    let mut claudemd_total: usize = CLAUDEMD_WRAPPER;
    let mut seen_md_paths: HashSet<String> = HashSet::new();
    for item in &scope_items {
        if !is_claudemd_name(&item.name) {
            continue;
        }
        if !seen_md_paths.insert(item.path.clone()) {
            // Already counted this exact file under another category.
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

    // ── Categorised items ──
    //
    // Three tiers, per the researched Claude Code context model:
    //   • always_loaded_items — FULL content that's always injected:
    //       rules WITHOUT `paths:`, plus the MEMORY.md index (below).
    //   • metadata_items — always-on METADATA only: the skill/command
    //       listing (`"name": description`) + subagent listing. These
    //       roll up into the capped `skill_listing` / `agent_listing`
    //       scalars, NOT into a per-item sum.
    //   • deferred_items — on-invoke bodies: skill/command/agent bodies
    //       and `paths:`-scoped rules. NOT part of `used`.
    let mut always_loaded_items: Vec<BudgetItem> = Vec::new();
    let mut metadata_items: Vec<BudgetItem> = Vec::new();
    let mut deferred_items: Vec<BudgetItem> = Vec::new();
    let mut skill_listing_raw: usize = 0;
    let mut agent_listing: usize = 0;
    let mut has_skill_or_command = false;

    for item in &scope_items {
        match item.category.as_str() {
            "skill" => {
                let meta = read_skill_meta(&item.path, &item.description);
                // `disable-model-invocation: true` → the model can't see
                // or call it, so it costs ZERO always-on (no metadata, no
                // boilerplate, no deferred body).
                if !meta.model_invocable {
                    continue;
                }
                has_skill_or_command = true;
                let listing = format!("\"{}\": {}", item.name, cap_desc(&meta.description));
                let m = tokenizer::count_text(&listing);
                skill_listing_raw += m.tokens;
                metadata_items.push(BudgetItem {
                    category: "skill".into(),
                    name: item.name.clone(),
                    tokens: m.tokens,
                    measured: m.measured,
                });
                let body = count_item(item);
                if body.tokens > 0 {
                    deferred_items.push(BudgetItem {
                        category: "skill".into(),
                        name: item.name.clone(),
                        tokens: body.tokens,
                        measured: body.measured,
                    });
                }
            }
            "command" => {
                // Slash-commands are model-invocable skills now: listing
                // metadata always-on, body deferred.
                has_skill_or_command = true;
                let listing = format!("\"{}\": {}", item.name, cap_desc(&item.description));
                let m = tokenizer::count_text(&listing);
                skill_listing_raw += m.tokens;
                metadata_items.push(BudgetItem {
                    category: "command".into(),
                    name: item.name.clone(),
                    tokens: m.tokens,
                    measured: m.measured,
                });
                let body = count_item(item);
                if body.tokens > 0 {
                    deferred_items.push(BudgetItem {
                        category: "command".into(),
                        name: item.name.clone(),
                        tokens: body.tokens,
                        measured: body.measured,
                    });
                }
            }
            "agent" => {
                // Subagents surface as an Agent-tool listing (name +
                // description); the body lives in the subagent's OWN
                // window, so it's deferred here.
                let listing = format!("{}: {}", item.name, cap_desc(&item.description));
                let m = tokenizer::count_text(&listing);
                agent_listing += m.tokens;
                metadata_items.push(BudgetItem {
                    category: "agent".into(),
                    name: item.name.clone(),
                    tokens: m.tokens,
                    measured: m.measured,
                });
                let body = count_item(item);
                if body.tokens > 0 {
                    deferred_items.push(BudgetItem {
                        category: "agent".into(),
                        name: item.name.clone(),
                        tokens: body.tokens,
                        measured: body.measured,
                    });
                }
            }
            "rule" => {
                // Rules are injected with HTML comments stripped. Those
                // WITHOUT `paths:` frontmatter load at session start
                // (always-on); `paths:`-scoped rules load only when the
                // model reads a matching file (deferred).
                let raw = std::fs::read_to_string(&item.path).unwrap_or_default();
                let cleaned = strip_html_comments(&raw);
                let m = tokenizer::count_text(&cleaned);
                if m.tokens == 0 {
                    continue;
                }
                let row = BudgetItem {
                    category: "rule".into(),
                    name: item.name.clone(),
                    tokens: m.tokens,
                    measured: m.measured,
                };
                if frontmatter_has_paths(&raw) {
                    deferred_items.push(row);
                } else {
                    always_loaded_items.push(row);
                }
            }
            _ => {}
        }
    }

    // ── MEMORY.md index (always-on, first 200 lines / 25 KB) ──
    // The scanner deliberately excludes MEMORY.md from the item list
    // (it isn't user-managed like the topic files), so we read it here
    // straight from the scope's memory dir — porting CCO's
    // `addMemoryIndexFiles`.
    if let Some(mem_path) = memory_index_path(home, scope_id) {
        if let Ok(count) = count_memory_index(&mem_path) {
            if count.tokens > 0 {
                always_loaded_items.push(BudgetItem {
                    category: "memory".into(),
                    name: "MEMORY.md".into(),
                    tokens: count.tokens,
                    measured: count.measured,
                });
            }
        }
    }

    // ── Skill/command listing cap + boilerplate ──
    // The whole listing is capped at a fraction of the context limit
    // (Claude Code truncates it), so many tiny descriptions can't run
    // away. Boilerplate is the one-time `<available_skills>` block.
    let skill_listing_budget = context_limit * SKILL_LISTING_BUDGET_PCT / 100;
    let skill_listing = skill_listing_raw.min(skill_listing_budget);
    let skill_boilerplate = if has_skill_or_command { SKILL_BOILERPLATE } else { 0 };

    // ── Active output style (folds into system overhead) ──
    let output_style = active_output_style_tokens(home, scope_id);

    // ── Totals ──
    // Always-on `used` = system + output style + CLAUDE.md + the capped
    // skill/agent listings + the always-on MCP tool-names line + the
    // full-content always-loaded items (unscoped rules + MEMORY.md).
    // MCP tool schemas + skill/command/agent bodies + scoped rules are
    // DEFERRED and surfaced separately.
    let loaded_subtotal: usize = always_loaded_items.iter().map(|i| i.tokens).sum();
    let used = SYSTEM_LOADED
        + output_style
        + mcp_tool_names
        + claudemd_total
        + skill_listing
        + skill_boilerplate
        + agent_listing
        + loaded_subtotal;
    let deferred_bodies: usize = deferred_items.iter().map(|i| i.tokens).sum();
    let deferred_total = SYSTEM_DEFERRED + mcp_schemas + deferred_bodies;

    BudgetBreakdown {
        system_loaded: SYSTEM_LOADED,
        output_style,
        system_deferred: SYSTEM_DEFERRED,
        mcp_schemas,
        mcp_tool_names,
        claudemd: claudemd_total,
        claude_md_files,
        skill_listing,
        skill_listing_raw,
        skill_boilerplate,
        agent_listing,
        always_loaded_items,
        metadata_items,
        deferred_items,
        deferred_total,
        autocompact_buffer: AUTOCOMPACT_BUFFER,
        max_output: MAX_OUTPUT,
        warning_threshold: WARNING_THRESHOLD,
        measured: tokenizer::active_tokenizer() == crate::tokenizer::TokenizerKind::Tiktoken,
        used,
        context_limit,
    }
}

/// Unique names of the *enabled* MCP servers for a scope, sorted. A
/// server is excluded when its own config carries `disabled: true` OR
/// its name appears in `disabled_names` (the settings-level
/// `disabledMcpjsonServers` list). This is what the budget composer
/// should be handed as `mcp_servers` — only enabled servers cost tokens.
pub fn enabled_mcp_servers(
    items: &[HarnessItem],
    scope_id: &str,
    disabled_names: &HashSet<String>,
) -> Vec<String> {
    let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for item in items {
        if item.category != "mcp" || item.scope_id != scope_id {
            continue;
        }
        if disabled_names.contains(&item.name) {
            continue;
        }
        let disabled = item
            .mcp_config
            .as_ref()
            .and_then(|c| c.get("disabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if disabled {
            continue;
        }
        set.insert(item.name.clone());
    }
    set.into_iter().collect()
}

/// Returns true when `name` is an ancestor-hierarchy memory file the
/// loader injects at session start: the root/repo `CLAUDE.md`, the
/// project-local `CLAUDE.local.md`, an enterprise-managed variant, or
/// the `.claude/CLAUDE.md` (scanned under `config`). Nested/subdir
/// CLAUDE.md files are NOT ancestor memory and never match here (the
/// scanner only emits ancestor files, so this is a belt-and-braces
/// guard).
fn is_claudemd_name(name: &str) -> bool {
    matches!(
        name,
        "CLAUDE.md"
            | ".claude/CLAUDE.md"
            | "CLAUDE.local.md"
            | ".claude/CLAUDE.local.md"
            | "CLAUDE.md (managed)"
    )
}

/// Read a CLAUDE.md, expand its `@import` lines, strip block-level HTML
/// comments, and tokenize — the exact transform Claude Code applies
/// before injecting the file. The `@import` recursion is bounded by
/// `MAX_IMPORT_DEPTH` and circular imports are detected via the `seen`
/// set seeded with the file itself.
fn count_claudemd(path: &Path, home: &Path) -> Result<TokenCount, WardError> {
    let raw = std::fs::read_to_string(path)?;
    let parent = path.parent().unwrap_or(home);
    let mut seen: HashSet<PathBuf> = HashSet::new();
    seen.insert(normalize_path(path));
    let expanded = expand_imports(&raw, parent, 0, &mut seen, home);
    let cleaned = strip_html_comments(&expanded);
    Ok(tokenizer::count_text(&cleaned))
}

/// Resolve the `MEMORY.md` index path for a scope. Global lives at
/// `~/.claude/memory/MEMORY.md`; a project scope (whose id is the
/// encoded project-dir name) lives at
/// `~/.claude/projects/<id>/memory/MEMORY.md`. Mirrors CCO's
/// `addMemoryIndexFiles`.
fn memory_index_path(home: &Path, scope_id: &str) -> Option<PathBuf> {
    if scope_id == "global" {
        Some(home.join(".claude").join("memory").join("MEMORY.md"))
    } else {
        Some(
            home.join(".claude")
                .join("projects")
                .join(scope_id)
                .join("memory")
                .join("MEMORY.md"),
        )
    }
}

/// Count a `MEMORY.md` index the way Claude Code loads it: only the
/// first `MEMORY_MAX_LINES` lines, then truncated to `MEMORY_MAX_BYTES`.
/// The per-topic auto-memory files it links to are loaded on demand and
/// are NOT counted here.
fn count_memory_index(path: &Path) -> Result<TokenCount, WardError> {
    let raw = std::fs::read_to_string(path)?;
    let mut capped: String = raw
        .lines()
        .take(MEMORY_MAX_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    if capped.len() > MEMORY_MAX_BYTES {
        // Truncate on a char boundary at/below the byte cap.
        let mut end = MEMORY_MAX_BYTES;
        while end > 0 && !capped.is_char_boundary(end) {
            end -= 1;
        }
        capped.truncate(end);
    }
    Ok(tokenizer::count_text(&capped))
}

/// Tokens contributed by an *active, non-default* output style. Claude
/// Code folds the selected style's markdown into the system prompt, so
/// a custom style adds always-on overhead. Returns 0 for the built-in
/// default (or when no style file is found). Reads the scope's
/// `settings.local.json` then `settings.json` for `outputStyle`, then
/// counts `~/.claude/output-styles/<name>.md` if present.
fn active_output_style_tokens(home: &Path, scope_id: &str) -> usize {
    // Only the global scope has a settings dir we can resolve from
    // `home` + `scope_id` alone; project settings live in the repo which
    // the composer isn't handed. Global is where the user's real config
    // lives, which is the case that matters.
    if scope_id != "global" {
        return 0;
    }
    let claude = home.join(".claude");
    let mut style_name: Option<String> = None;
    // Local settings win over shared settings.
    for f in ["settings.local.json", "settings.json"] {
        let p = claude.join(f);
        let Ok(content) = std::fs::read_to_string(&p) else { continue };
        let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&content) else { continue };
        if let Some(v) = cfg.get("outputStyle").and_then(|v| v.as_str()) {
            style_name = Some(v.to_string());
            break;
        }
    }
    let name = match style_name {
        Some(n) => n,
        None => return 0,
    };
    // The built-in default carries no extra always-on text.
    if name.is_empty() || name.eq_ignore_ascii_case("default") {
        return 0;
    }
    // Custom styles live as markdown files; count the file if present.
    // Built-in named styles (no file) can't be measured — treat as 0.
    let style_file = claude.join("output-styles").join(format!("{name}.md"));
    match tokenizer::count_file(&style_file) {
        Ok(c) => c.tokens,
        Err(_) => 0,
    }
}

/// Tokenize one item's FULL file body (used for deferred skill/command/
/// agent bodies). Missing/empty → 0 tokens.
fn count_item(item: &HarnessItem) -> TokenCount {
    if item.path.is_empty() {
        return TokenCount { tokens: 0, measured: false };
    }
    match tokenizer::count_file(Path::new(&item.path)) {
        Ok(c) => c,
        Err(_) => TokenCount { tokens: 0, measured: false },
    }
}

/// Frontmatter-derived facts about a skill relevant to the budget.
struct SkillMeta {
    /// False when `disable-model-invocation: true` — the model can't see
    /// or call the skill, so it costs zero always-on.
    model_invocable: bool,
    /// The skill's `description` (falls back to the scanned description).
    description: String,
}

/// Read a `SKILL.md`'s frontmatter for the fields the budget needs. Falls
/// back to `scanned_desc` when the file is unreadable or omits the field.
fn read_skill_meta(path: &str, scanned_desc: &str) -> SkillMeta {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let fm = crate::fs_utils::parse_frontmatter(&content);
    let model_invocable = fm
        .get("disable-model-invocation")
        .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "yes" | "1"))
        .unwrap_or(true);
    let description = match fm.get("description") {
        Some(d) if !d.trim().is_empty() => d.trim().to_string(),
        _ => scanned_desc.to_string(),
    };
    SkillMeta { model_invocable, description }
}

/// Truncate a listing description to `SKILL_DESC_CAP` characters, on a
/// char boundary (Claude Code caps long descriptions in the listing).
fn cap_desc(desc: &str) -> String {
    if desc.chars().count() <= SKILL_DESC_CAP {
        return desc.to_string();
    }
    desc.chars().take(SKILL_DESC_CAP).collect()
}

/// True when a markdown file's leading `---` frontmatter block carries a
/// top-level `paths:` key (a path-scoped rule that loads on demand, not
/// at session start). Mirrors CCO's `^paths:` frontmatter check.
fn frontmatter_has_paths(content: &str) -> bool {
    let trimmed = content.trim_start_matches('\u{feff}');
    let stripped = match trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
    {
        Some(s) => s,
        None => return false,
    };
    let end = match stripped.find("\n---").or_else(|| stripped.find("\r\n---")) {
        Some(i) => i,
        None => return false,
    };
    stripped[..end]
        .lines()
        .any(|l| l.trim_end_matches('\r').starts_with("paths:"))
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
            modified_ms: None,
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
    fn compose_places_skill_command_agent_metadata_rule_full() {
        // New model: only rules WITHOUT `paths:` are full always-on;
        // skills/commands/agents surface as METADATA rows; their bodies
        // are deferred. Arbitrary "memory" topic items are ignored (only
        // the MEMORY.md index counts).
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("skill.md");
        let rule_path = dir.path().join("rule.md");
        let cmd_path = dir.path().join("cmd.md");
        let agent_path = dir.path().join("agent.md");
        let memory_path = dir.path().join("mem.md");
        fs::write(&skill_path, "skill body 1234").unwrap();
        fs::write(&rule_path, "rule body 12345").unwrap();
        fs::write(&cmd_path, "cmd body 123456").unwrap();
        fs::write(&agent_path, "agent body 1234567").unwrap();
        fs::write(&memory_path, "mem").unwrap();

        let items = vec![
            item("skill", "global", "skill", skill_path.to_str().unwrap()),
            item("rule", "global", "rule", rule_path.to_str().unwrap()),
            item("command", "global", "cmd", cmd_path.to_str().unwrap()),
            item("agent", "global", "agent", agent_path.to_str().unwrap()),
            item("memory", "global", "mem", memory_path.to_str().unwrap()),
        ];
        let b = compose("global", &items, &[], dir.path());
        // Always-loaded FULL: just the unscoped rule.
        let loaded_cats: Vec<&str> = b.always_loaded_items.iter().map(|i| i.category.as_str()).collect();
        assert_eq!(loaded_cats, vec!["rule"]);
        // Metadata: skill + command + agent.
        let mut meta_cats: Vec<&str> = b.metadata_items.iter().map(|i| i.category.as_str()).collect();
        meta_cats.sort();
        assert_eq!(meta_cats, vec!["agent", "command", "skill"]);
        // Deferred: skill + command + agent bodies (rule has no paths:).
        let mut def_cats: Vec<&str> = b.deferred_items.iter().map(|i| i.category.as_str()).collect();
        def_cats.sort();
        assert_eq!(def_cats, vec!["agent", "command", "skill"]);
        // No arbitrary memory topic file leaks into any tier.
        assert!(!loaded_cats.contains(&"memory"));
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
        // Always-on `used` = system + MCP tool-NAMES line + wrapper
        // (no claudemd files, 0 always-loaded). MCP schemas are DEFERRED.
        assert_eq!(b.used, SYSTEM_LOADED + MCP_TOOL_NAMES + CLAUDEMD_WRAPPER);
        assert_eq!(b.deferred_total, SYSTEM_DEFERRED + MCP_TOOL_SCHEMA);
    }

    #[test]
    fn compose_filters_to_requested_scope_only() {
        let dir = tempfile::tempdir().unwrap();
        // Use rules (full always-on) so we can assert on always_loaded_items.
        let p = dir.path().join("a.md");
        fs::write(&p, "x".repeat(40)).unwrap(); // 10 tokens
        let items = vec![
            item("rule", "scope-a", "a-rule", p.to_str().unwrap()),
            item("rule", "scope-b", "b-rule", p.to_str().unwrap()),
        ];
        let b_a = compose("scope-a", &items, &[], dir.path());
        let b_b = compose("scope-b", &items, &[], dir.path());
        assert_eq!(b_a.always_loaded_items.len(), 1);
        assert_eq!(b_b.always_loaded_items.len(), 1);
        assert_eq!(b_a.always_loaded_items[0].name, "a-rule");
        assert_eq!(b_b.always_loaded_items[0].name, "b-rule");
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
        // Top-level camelCase fields, including the new tiered ones.
        for key in [
            "systemLoaded", "outputStyle", "mcpSchemas", "mcpToolNames",
            "alwaysLoadedItems", "metadataItems", "deferredItems", "deferredTotal",
            "skillListing", "skillListingRaw", "skillBoilerplate", "agentListing",
            "autocompactBuffer", "warningThreshold", "contextLimit",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
    }

    // ── Commit 1: CLAUDE.md refinements + MEMORY.md + output styles ──

    #[test]
    fn compose_counts_memory_index_always_on() {
        // MEMORY.md at ~/.claude/memory/MEMORY.md must be counted as an
        // always-on item, so `used` grows beyond the empty baseline.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let mem = home.join(".claude/memory/MEMORY.md");
        fs::create_dir_all(mem.parent().unwrap()).unwrap();
        fs::write(&mem, "index line one\nindex line two\n").unwrap();
        let items: Vec<HarnessItem> = vec![];
        let b = compose("global", &items, &[], home);
        assert!(
            b.used > SYSTEM_LOADED + CLAUDEMD_WRAPPER,
            "MEMORY.md must add to always-on used; got {}",
            b.used
        );
    }

    #[test]
    fn expand_imports_skips_at_import_inside_code_fence() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("frag.md"), "SHOULD-NOT-APPEAR").unwrap();
        let home = dir.path();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        // The @frag.md sits inside a fenced code block and must be kept
        // verbatim (Claude Code does not expand imports inside fences).
        let src = "```\n@frag.md\n```";
        let out = expand_imports(src, dir.path(), 0, &mut seen, home);
        assert_eq!(out, src);
        assert!(!out.contains("SHOULD-NOT-APPEAR"));
    }

    #[test]
    fn expand_imports_skips_inline_code_span() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("frag.md"), "SHOULD-NOT-APPEAR").unwrap();
        let home = dir.path();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        // A line that opens with a backtick is an inline code span.
        let src = "`@frag.md` is how you import";
        let out = expand_imports(src, dir.path(), 0, &mut seen, home);
        assert_eq!(out, src);
    }

    #[test]
    fn max_import_depth_is_four() {
        assert_eq!(MAX_IMPORT_DEPTH, 4);
    }

    #[test]
    fn strip_html_comments_removes_block_comments() {
        assert_eq!(strip_html_comments("a<!-- hide -->b"), "ab");
        // Multi-line.
        assert_eq!(strip_html_comments("x<!--\nmany\nlines\n-->y"), "xy");
        // Unterminated — everything after the open is dropped.
        assert_eq!(strip_html_comments("keep<!-- oops"), "keep");
        // No comment — unchanged.
        assert_eq!(strip_html_comments("plain text"), "plain text");
    }

    #[test]
    fn count_claudemd_strips_html_comments_before_counting() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let with_comment = home.join("WITH.md");
        let without = home.join("WITHOUT.md");
        // Identical visible text; one carries a big HTML comment that
        // must NOT be tokenized.
        fs::write(&with_comment, "hello<!-- 0123456789012345678901234567890123456789 -->world").unwrap();
        fs::write(&without, "helloworld").unwrap();
        let a = count_claudemd(&with_comment, home).unwrap();
        let b = count_claudemd(&without, home).unwrap();
        assert_eq!(a.tokens, b.tokens, "HTML comment must be stripped before counting");
    }

    #[test]
    fn compose_dedupes_claudemd_counted_under_two_categories() {
        // The scanner emits the SAME CLAUDE.md under both `memory` and
        // `config`. It must be counted once, not twice.
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let md = home.join(".claude/CLAUDE.md");
        fs::create_dir_all(md.parent().unwrap()).unwrap();
        fs::write(&md, "x".repeat(400)).unwrap(); // 100 tokens
        let items = vec![
            item("memory", "global", "CLAUDE.md", md.to_str().unwrap()),
            item("config", "global", "CLAUDE.md", md.to_str().unwrap()),
        ];
        let b = compose("global", &items, &[], home);
        // Only one CLAUDE.md file row; claudemd = 100 content + wrapper.
        assert_eq!(b.claude_md_files.len(), 1);
        assert_eq!(b.claudemd, 100 + CLAUDEMD_WRAPPER);
    }

    #[test]
    fn compose_counts_active_output_style_into_used() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let claude = home.join(".claude");
        fs::create_dir_all(claude.join("output-styles")).unwrap();
        fs::write(claude.join("settings.json"), r#"{"outputStyle":"verbose"}"#).unwrap();
        fs::write(claude.join("output-styles/verbose.md"), "s".repeat(400)).unwrap(); // 100 tokens
        let items: Vec<HarnessItem> = vec![];
        let b = compose("global", &items, &[], home);
        assert_eq!(b.output_style, 100);
        assert_eq!(b.used, SYSTEM_LOADED + 100 + CLAUDEMD_WRAPPER);
    }

    #[test]
    fn default_output_style_costs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let claude = home.join(".claude");
        fs::create_dir_all(&claude).unwrap();
        fs::write(claude.join("settings.json"), r#"{"outputStyle":"default"}"#).unwrap();
        let items: Vec<HarnessItem> = vec![];
        let b = compose("global", &items, &[], home);
        assert_eq!(b.output_style, 0);
    }

    #[test]
    fn memory_index_capped_at_200_lines() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let mem = home.join(".claude/memory/MEMORY.md");
        fs::create_dir_all(mem.parent().unwrap()).unwrap();
        // 300 lines of "ABC" (4 bytes each incl. newline). Only the
        // first 200 count.
        let body = "ABC\n".repeat(300);
        fs::write(&mem, &body).unwrap();
        let items: Vec<HarnessItem> = vec![];
        let b = compose("global", &items, &[], home);
        let mem_item = b.always_loaded_items.iter().find(|i| i.category == "memory").unwrap();
        // 200 lines joined by "\n" = 200*3 + 199 = 799 bytes -> 200 tokens.
        assert_eq!(mem_item.tokens, 200);
    }

    #[test]
    fn compose_scales_context_limit_for_million_token_model() {
        let dir = tempfile::tempdir().unwrap();
        let b = compose_with_limit("global", &[], &[], dir.path(), 1_000_000);
        assert_eq!(b.context_limit, 1_000_000);
    }

    // ── Commit 2: skills/commands/agents metadata + rules paths: split ──

    /// Helper: write a SKILL.md with a frontmatter description + a huge
    /// body, and return an item pointing at it.
    fn write_skill(dir: &Path, name: &str, desc: &str, body_bytes: usize, disable: bool) -> HarnessItem {
        let skill_dir = dir.join(".claude/skills").join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let mut fm = format!("---\nname: {name}\ndescription: {desc}\n");
        if disable {
            fm.push_str("disable-model-invocation: true\n");
        }
        fm.push_str("---\n");
        let body = "B".repeat(body_bytes);
        let manifest = skill_dir.join("SKILL.md");
        fs::write(&manifest, format!("{fm}{body}")).unwrap();
        item("skill", "global", name, manifest.to_str().unwrap())
    }

    #[test]
    fn skill_large_body_counts_as_metadata_not_body() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        // 8000-byte body (~2000 tokens) but a short description.
        let it = write_skill(home, "big", "short summary", 8000, false);
        let b = compose("global", &[it], &[], home);
        // Skill contributes a tiny metadata row, not the 2000-token body.
        let meta = b.metadata_items.iter().find(|i| i.category == "skill").unwrap();
        assert!(meta.tokens < 50, "metadata should be tiny; got {}", meta.tokens);
        // The body is DEFERRED, not always-on.
        let def = b.deferred_items.iter().find(|i| i.category == "skill").unwrap();
        assert!(def.tokens > 1500, "body should be deferred; got {}", def.tokens);
        // `used` excludes the body entirely (only tiny metadata + the
        // one-time boilerplate ride along).
        assert!(
            b.used < SYSTEM_LOADED + CLAUDEMD_WRAPPER + SKILL_BOILERPLATE + 50,
            "skill body must not inflate always-on used; got {}",
            b.used
        );
        assert!(b.skill_boilerplate == SKILL_BOILERPLATE);
    }

    #[test]
    fn disable_model_invocation_skill_counts_zero_always_on() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let it = write_skill(home, "hidden", "not invocable", 8000, true);
        let b = compose("global", &[it], &[], home);
        // No metadata, no boilerplate, no body — pure baseline.
        assert_eq!(b.used, SYSTEM_LOADED + CLAUDEMD_WRAPPER);
    }

    #[test]
    fn paths_scoped_rule_is_deferred_not_always_on() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let rule = home.join(".claude/rules/scoped.md");
        fs::create_dir_all(rule.parent().unwrap()).unwrap();
        // A big paths:-scoped rule — must NOT count toward always-on used.
        fs::write(&rule, format!("---\npaths:\n  - \"*.py\"\n---\n{}", "R".repeat(4000))).unwrap();
        let it = item("rule", "global", "scoped", rule.to_str().unwrap());
        let b = compose("global", &[it], &[], home);
        assert_eq!(b.used, SYSTEM_LOADED + CLAUDEMD_WRAPPER, "scoped rule must be deferred: {}", b.used);
        // And it lands in the deferred bucket.
        assert!(b.deferred_items.iter().any(|i| i.category == "rule" && i.name == "scoped"));
        assert!(b.deferred_total > SYSTEM_DEFERRED);
    }

    #[test]
    fn unscoped_rule_stays_always_on_full() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let rule = home.join("r.md");
        fs::write(&rule, format!("---\ndescription: x\n---\n{}", "R".repeat(400))).unwrap();
        let it = item("rule", "global", "plain", rule.to_str().unwrap());
        let b = compose("global", &[it], &[], home);
        assert!(b.always_loaded_items.iter().any(|i| i.category == "rule" && i.name == "plain"));
        assert!(b.used > SYSTEM_LOADED + CLAUDEMD_WRAPPER);
    }

    #[test]
    fn command_body_is_deferred_metadata_is_always_on() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cmd = home.join(".claude/commands/deploy.md");
        fs::create_dir_all(cmd.parent().unwrap()).unwrap();
        fs::write(&cmd, "R".repeat(4000)).unwrap(); // 1000-token body
        let mut it = item("command", "global", "deploy", cmd.to_str().unwrap());
        it.description = "Ship the app to prod".into();
        let b = compose("global", &[it], &[], home);
        // Metadata tiny + folded into the capped skill listing.
        assert!(b.metadata_items.iter().any(|i| i.category == "command"));
        assert!(b.skill_listing > 0 && b.skill_listing < 50);
        // Body deferred, not in used.
        assert!(b.deferred_items.iter().any(|i| i.category == "command"));
        assert!(b.used < SYSTEM_LOADED + CLAUDEMD_WRAPPER + SKILL_BOILERPLATE + 50);
    }

    #[test]
    fn agent_body_is_deferred_metadata_in_agent_listing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let agent = home.join(".claude/agents/reviewer.md");
        fs::create_dir_all(agent.parent().unwrap()).unwrap();
        fs::write(&agent, "R".repeat(4000)).unwrap(); // 1000-token body
        let mut it = item("agent", "global", "reviewer", agent.to_str().unwrap());
        it.description = "Reviews diffs for bugs".into();
        let b = compose("global", &[it], &[], home);
        // Agent metadata rolls into `agent_listing`, NOT `skill_listing`.
        assert!(b.agent_listing > 0);
        assert_eq!(b.skill_listing, 0);
        // No skill boilerplate for an agent-only scope.
        assert_eq!(b.skill_boilerplate, 0);
        // Body deferred.
        assert!(b.deferred_items.iter().any(|i| i.category == "agent"));
        assert!(b.used < SYSTEM_LOADED + CLAUDEMD_WRAPPER + 50);
    }

    #[test]
    fn skill_listing_is_capped_at_one_percent_of_limit() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        // 400 skills each with a long (capped) description — uncapped this
        // would blow past 1% of 200K (=2000). The listing must be capped.
        let long_desc = "d".repeat(1200);
        let mut items = Vec::new();
        for i in 0..400 {
            items.push(write_skill(home, &format!("skill{i}"), &long_desc, 10, false));
        }
        let b = compose("global", &items, &[], home);
        let budget = DEFAULT_CONTEXT_LIMIT * SKILL_LISTING_BUDGET_PCT / 100; // 2000
        assert_eq!(b.skill_listing, budget, "listing must be capped at 1%");
        assert!(b.skill_listing_raw > budget, "raw sum should exceed the cap");
    }

    #[test]
    fn skill_listing_budget_scales_with_context_limit() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let long_desc = "d".repeat(1200);
        let mut items = Vec::new();
        for i in 0..400 {
            items.push(write_skill(home, &format!("skill{i}"), &long_desc, 10, false));
        }
        // 1% of a 1M model = 10,000 — a bigger cap than the 200K case.
        let b = compose_with_limit("global", &items, &[], home, 1_000_000);
        assert_eq!(b.skill_listing, 1_000_000 / 100);
    }

    // ── Commit 3: MCP schemas -> deferred + always-on tool-names line ──

    #[test]
    fn mcp_schemas_land_in_deferred_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let b = compose("global", &[], &["s1".into()], home);
        // The 3100-token schema must NOT be part of always-on `used`.
        assert!(
            b.used < SYSTEM_LOADED + 1000,
            "mcp schemas must not inflate used; got {}",
            b.used
        );
        // It rides in the deferred figure alongside the deferred system
        // tools.
        assert_eq!(b.deferred_total, SYSTEM_DEFERRED + MCP_TOOL_SCHEMA);
        // The short always-on tool-names line IS counted (once).
        assert_eq!(b.mcp_tool_names, MCP_TOOL_NAMES);
        assert_eq!(b.used, SYSTEM_LOADED + CLAUDEMD_WRAPPER + MCP_TOOL_NAMES);
    }

    #[test]
    fn no_enabled_servers_means_no_tool_names_line() {
        let dir = tempfile::tempdir().unwrap();
        let b = compose("global", &[], &[], dir.path());
        assert_eq!(b.mcp_tool_names, 0);
        assert_eq!(b.mcp_schemas, 0);
    }

    #[test]
    fn enabled_mcp_servers_excludes_disabled() {
        let mut disabled_flag = serde_json::Map::new();
        disabled_flag.insert("command".into(), serde_json::json!("x"));
        disabled_flag.insert("disabled".into(), serde_json::json!(true));
        let items = vec![
            {
                let mut it = item("mcp", "global", "github", "/p");
                it.mcp_config = Some(serde_json::json!({"command": "gh"}));
                it
            },
            {
                // Per-server disabled flag.
                let mut it = item("mcp", "global", "off-server", "/p");
                it.mcp_config = Some(serde_json::Value::Object(disabled_flag));
                it
            },
            {
                // Disabled via the settings-level list.
                let mut it = item("mcp", "global", "listed-off", "/p");
                it.mcp_config = Some(serde_json::json!({"command": "x"}));
                it
            },
            // Wrong scope — excluded.
            item("mcp", "other", "elsewhere", "/p"),
        ];
        let mut disabled_names: HashSet<String> = HashSet::new();
        disabled_names.insert("listed-off".into());
        let enabled = enabled_mcp_servers(&items, "global", &disabled_names);
        assert_eq!(enabled, vec!["github".to_string()]);
    }
}