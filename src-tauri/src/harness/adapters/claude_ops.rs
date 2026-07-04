//! claude_ops.rs — Move / delete / restore / save-file for the Claude
//! adapter. Port of CCO's `claude-operations.mjs` to Rust.
//!
//! Rules:
//!   - memory, skill, command, agent → can move between global and any
//!     project scope, but NOT to a project whose `.claude` IS the global
//!     `~/.claude` (home-overlap is rejected for file-based categories).
//!   - mcp → can move to ANY scope (including home-overlap), because
//!     MCP entries are stored in `~/.claude/.mcp.json` / `~/.mcp.json` /
//!     `~/.claude.json` / per-repo `.mcp.json`, not in `.claude/skills/`.
//!   - plan / rule / config / hook / plugin / session / setting →
//     locked (no destinations).
//!   - `item.locked` always rejects.
//!
//! MCP move is a **JSON edit** (delete from source object, insert into
//! destination object) — never a file rename.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::WardError;
use crate::fs_utils::ensure_under_home;
use crate::harness::{Ctx, HarnessOps};
use crate::model::{Destination, HarnessItem, RestoreInfo, Scope};

// ── Share detection ────────────────────────────────────────────────────

/// True when `scope`'s `.claude` directory IS the global `~/.claude`.
/// This happens when `scope.root == home` (the user opened Ward in their
/// own home directory). For file-based categories we hide these scopes
/// from `get_valid_destinations` because moving there would land the
/// file under `~/.claude/` — same place as the global scope.
pub fn shares_global_claude_dir(home: &Path, scope: &Scope) -> bool {
    if scope.id == "global" {
        return false; // global is never "overlapping itself" — it's the source.
    }
    if scope.kind != "project" && scope.kind != "project-unresolved" {
        return false;
    }
    let repo = PathBuf::from(&scope.root);
    repo.join(".claude") == home.join(".claude")
}

// ── Per-scope dir resolvers ────────────────────────────────────────────

fn claude_root(home: &Path) -> PathBuf { home.join(".claude") }

fn projects_dir(home: &Path) -> PathBuf { claude_root(home).join("projects") }

pub fn resolve_memory_dir(scope_id: &str, home: &Path) -> PathBuf {
    if scope_id == "global" {
        claude_root(home).join("memory")
    } else {
        projects_dir(home).join(scope_id).join("memory")
    }
}

pub fn resolve_plan_dir(scope_id: &str, home: &Path) -> PathBuf {
    if scope_id == "global" {
        claude_root(home).join("plans")
    } else {
        projects_dir(home).join(scope_id).join("plans")
    }
}

fn find_scope<'a>(scopes: &'a [Scope], scope_id: &str) -> Option<&'a Scope> {
    scopes.iter().find(|s| s.id == scope_id)
}

pub fn resolve_skill_dir(scope_id: &str, scopes: &[Scope]) -> Option<PathBuf> {
    if scope_id == "global" {
        return Some(claude_root_for_scope_id(scope_id, scopes).join("skills"));
    }
    let scope = find_scope(scopes, scope_id)?;
    let repo = PathBuf::from(&scope.root);
    if scope.kind != "project" { return None; }
    Some(repo.join(".claude").join("skills"))
}

pub fn resolve_command_dir(scope_id: &str, scopes: &[Scope]) -> Option<PathBuf> {
    if scope_id == "global" {
        return Some(claude_root_for_scope_id(scope_id, scopes).join("commands"));
    }
    let scope = find_scope(scopes, scope_id)?;
    let repo = PathBuf::from(&scope.root);
    if scope.kind != "project" { return None; }
    Some(repo.join(".claude").join("commands"))
}

pub fn resolve_agent_dir(scope_id: &str, scopes: &[Scope]) -> Option<PathBuf> {
    if scope_id == "global" {
        return Some(claude_root_for_scope_id(scope_id, scopes).join("agents"));
    }
    let scope = find_scope(scopes, scope_id)?;
    let repo = PathBuf::from(&scope.root);
    if scope.kind != "project" { return None; }
    Some(repo.join(".claude").join("agents"))
}

pub fn resolve_rule_dir(scope_id: &str, scopes: &[Scope]) -> Option<PathBuf> {
    if scope_id == "global" {
        return Some(claude_root_for_scope_id(scope_id, scopes).join("rules"));
    }
    let scope = find_scope(scopes, scope_id)?;
    let repo = PathBuf::from(&scope.root);
    if scope.kind != "project" { return None; }
    Some(repo.join(".claude").join("rules"))
}

pub fn resolve_mcp_json(scope_id: &str, scopes: &[Scope]) -> Option<PathBuf> {
    if scope_id == "global" {
        return Some(claude_root_for_scope_id(scope_id, scopes).join(".mcp.json"));
    }
    let scope = find_scope(scopes, scope_id)?;
    let repo = PathBuf::from(&scope.root);
    if scope.kind != "project" { return None; }
    Some(repo.join(".mcp.json"))
}

/// Look up the `.claude` root that backs `scope_id`. For `global` this
/// is `home/.claude`; for project scopes it's `repo/.claude` (resolved
/// from `scopes`).
fn claude_root_for_scope_id(scope_id: &str, scopes: &[Scope]) -> PathBuf {
    if scope_id == "global" {
        // CCO uses `homedir()`; we walk back via the only "global"
        // scope. Adapters put their `home` in the global scope's root
        // `parent`, so we replicate that by reading the parent of
        // `<global>/.claude`. To stay portable we accept that the
        // caller passes us `home` indirectly via `scopes` (the global
        // scope's `root` is `<home>/.claude`).
        if let Some(g) = find_scope(scopes, "global") {
            return PathBuf::from(&g.root);
        }
    }
    if let Some(scope) = find_scope(scopes, scope_id) {
        return PathBuf::from(&scope.root).join(".claude");
    }
    PathBuf::new()
}

// ── Validation ────────────────────────────────────────────────────────

pub fn validate_move(item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope])
    -> Result<(), WardError>
{
    if item.locked {
        return Err(WardError::NotFound(format!("{} is locked and cannot be moved", item.name)));
    }
    if item.scope_id == dest_scope_id {
        return Err(WardError::NotFound("Item is already in this scope".into()));
    }
    if !matches!(item.category.as_str(),
        "memory" | "skill" | "mcp" | "plan" | "command" | "agent" | "rule")
    {
        return Err(WardError::NotFound(format!("{} items cannot be moved", item.category)));
    }
    if find_scope(scopes, dest_scope_id).is_none() {
        return Err(WardError::NotFound(format!("Unknown scope: {dest_scope_id}")));
    }
    Ok(())
}

// ── get_valid_destinations ─────────────────────────────────────────────

/// Build the list of scopes a user can move `item` to. The single
/// parity-critical function for the Organizer UI — ported exactly from
/// CCO's `getValidDestinations`.
pub fn get_valid_destinations(home: &Path, item: &HarnessItem, scopes: &[Scope]) -> Vec<Destination> {
    if item.locked { return vec![]; }
    scopes.iter()
        .filter(|s| s.id != item.scope_id)
        .filter(|s| match item.category.as_str() {
            "memory" | "skill" | "command" | "agent" => {
                // File-based items: global is always valid; project
                // scopes only when their `.claude` differs from global
                // (no home-overlap).
                s.id == "global" || (s.kind == "project" && !shares_global_claude_dir(home, s))
            }
            "mcp" => true, // MCP lives in .mcp.json, never in <repo>/.claude
            "plan" | "rule" | "config" | "hook" | "plugin" | "session" | "setting" => false,
            _ => false,
        })
        .map(|s| Destination {
            scope_id: s.id.clone(),
            label: s.label.clone(),
            kind: s.kind.clone(),
        })
        .collect()
}

// ── The HarnessOps implementation ──────────────────────────────────────

pub struct ClaudeOps;

impl HarnessOps for ClaudeOps {
    fn get_valid_destinations(&self, ctx: &Ctx, item: &HarnessItem, scopes: &[Scope]) -> Vec<Destination> {
        get_valid_destinations(ctx.home, item, scopes)
    }

    fn move_item(&self, ctx: &Ctx, item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope])
        -> Result<RestoreInfo, WardError>
    {
        validate_move(item, dest_scope_id, scopes)?;
        match item.category.as_str() {
            "memory" => move_memory(ctx, item, dest_scope_id, scopes),
            "skill" => move_skill(ctx, item, dest_scope_id, scopes),
            "plan" => move_plan(ctx, item, dest_scope_id, scopes),
            "rule" => move_rule(ctx, item, dest_scope_id, scopes),
            "command" => move_command(ctx, item, dest_scope_id, scopes),
            "agent" => move_agent(ctx, item, dest_scope_id, scopes),
            "mcp" => move_mcp(ctx, item, dest_scope_id, scopes),
            other => Err(WardError::NotFound(format!("{other} items cannot be moved"))),
        }
    }

    fn delete_item(&self, ctx: &Ctx, item: &HarnessItem, scopes: &[Scope])
        -> Result<RestoreInfo, WardError>
    {
        // Lock check (mirrors CCO deleteItem).
        if item.locked {
            return Err(WardError::NotFound(format!("{} is locked and cannot be deleted", item.name)));
        }
        if !matches!(item.category.as_str(),
            "memory" | "skill" | "mcp" | "plan" | "command" | "agent" | "rule" | "session")
        {
            return Err(WardError::NotFound(format!("{} items cannot be deleted", item.category)));
        }
        match item.category.as_str() {
            "memory" | "plan" | "command" | "agent" | "rule" => delete_single_file(ctx, item),
            "skill" => delete_skill_dir(ctx, item),
            "session" => delete_session(ctx, item),
            "mcp" => delete_mcp_entry(ctx, item),
            other => Err(WardError::NotFound(format!("{other} items cannot be deleted"))),
        }
    }

    fn restore(&self, ctx: &Ctx, info: &RestoreInfo) -> Result<(), WardError> {
        match info.kind.as_str() {
            "file" => restore_file(ctx, info),
            "mcp-entry" => restore_mcp_entry(ctx, info),
            other => Err(WardError::NotFound(format!("Unknown restore kind: {other}"))),
        }
    }

    fn save_file(&self, ctx: &Ctx, path: &str, content: &str) -> Result<(), WardError> {
        let p = Path::new(path);
        let abs = ensure_under_home(p, ctx.home)?;
        if let Some(parent) = abs.parent() { std::fs::create_dir_all(parent)?; }
        std::fs::write(&abs, content)?;
        Ok(())
    }
}

// ── Per-category move implementations ──────────────────────────────────

fn file_name_for_item(item: &HarnessItem) -> &str {
    // item.name for memory/plan/command/agent/rule is the display name
    // (from frontmatter or stem); item.path ends with the real file
    // name. We always want the actual on-disk filename.
    let p = Path::new(&item.path);
    p.file_name().and_then(|s| s.to_str()).unwrap_or(&item.name)
}

fn move_memory(_ctx: &Ctx, item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope])
    -> Result<RestoreInfo, WardError>
{
    let _scopes = scopes;
    // Memory files live at `~/.claude/memory/<name>.md` (global) or
    // `~/.claude/projects/<encoded>/memory/<name>.md` (project). We
    // rename the existing path into the destination dir.
    let to_dir = match dest_scope_id {
        // Global destination uses `home`-derived paths, but we don't
        // have `home` here — the harness ops receive it via Ctx. We
        // look up the destination scope's root and build the right
        // memory subdir.
        id => {
            let scope = scopes.iter().find(|s| s.id == id)
                .ok_or_else(|| WardError::NotFound(format!("Unknown scope: {id}")))?;
            if id == "global" {
                PathBuf::from(&scope.root).join("memory")
            } else if scope.kind == "project" {
                PathBuf::from(&scope.root).join("memory")
            } else {
                // project-unresolved: fall back to ~/.claude/projects/<id>/memory
                crate::fs_utils::ensure_under_home(
                    Path::new(&scope.root).join("memory").as_path(),
                    Path::new("/"),
                ).unwrap_or_else(|_| PathBuf::from(&scope.root).join("memory"))
            }
        }
    };
    let _to_dir = to_dir; // (unused beyond compile check; see move_memory_v2 below)
    move_single_md(item, dest_scope_id, scopes, "memory")
}

fn move_skill(_ctx: &Ctx, item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope])
    -> Result<RestoreInfo, WardError>
{
    move_skill_dir(item, dest_scope_id, scopes)
}

fn move_plan(_ctx: &Ctx, item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope])
    -> Result<RestoreInfo, WardError>
{
    move_single_md(item, dest_scope_id, scopes, "plan")
}

fn move_rule(_ctx: &Ctx, item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope])
    -> Result<RestoreInfo, WardError>
{
    move_single_md(item, dest_scope_id, scopes, "rule")
}

fn move_command(_ctx: &Ctx, item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope])
    -> Result<RestoreInfo, WardError>
{
    move_single_md(item, dest_scope_id, scopes, "command")
}

fn move_agent(_ctx: &Ctx, item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope])
    -> Result<RestoreInfo, WardError>
{
    move_single_md(item, dest_scope_id, scopes, "agent")
}

/// Rename a single .md file into the destination scope's category dir.
/// Uses CCO's `safeRename` (rename, fallback to copy+rm on EXDEV).
fn move_single_md(item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope], category: &str)
    -> Result<RestoreInfo, WardError>
{
    let to_dir = resolve_category_dir(dest_scope_id, category, scopes)
        .ok_or_else(|| WardError::NotFound(format!("Cannot resolve {category} dir for {dest_scope_id}")))?;
    let file_name = file_name_for_item(item);
    let to_path = to_dir.join(file_name);
    if to_path.exists() {
        return Err(WardError::NotFound(format!("{file_name} already exists at destination")));
    }
    std::fs::create_dir_all(&to_dir)?;
    let from_path = PathBuf::from(&item.path);
    safe_rename(&from_path, &to_path, false)?;
    Ok(RestoreInfo {
        kind: "file".into(),
        original_path: from_path.display().to_string(),
        current_path: Some(to_path.display().to_string()),
        backup_bytes: None,
        mcp_entry: None,
        mcp_key: None,
        mcp_parent_key: None,
        mcp_scope: None,
    })
}

fn resolve_category_dir(scope_id: &str, category: &str, scopes: &[Scope]) -> Option<PathBuf> {
    match category {
        "memory" => {
            if scope_id == "global" {
                Some(claude_root_for_scope_id(scope_id, scopes).join("memory"))
            } else {
                let scope = find_scope(scopes, scope_id)?;
                if scope.kind == "project" {
                    Some(PathBuf::from(&scope.root).join("memory"))
                } else {
                    Some(PathBuf::from(&scope.root).join("memory"))
                }
            }
        }
        "plan" => {
            if scope_id == "global" {
                Some(claude_root_for_scope_id(scope_id, scopes).join("plans"))
            } else {
                Some(claude_root_for_scope_id(scope_id, scopes).join("plans"))
            }
        }
        "rule" => resolve_rule_dir(scope_id, scopes).map(|p| p),
        "command" => resolve_command_dir(scope_id, scopes),
        "agent" => resolve_agent_dir(scope_id, scopes),
        "skill" => resolve_skill_dir(scope_id, scopes),
        _ => None,
    }
}

/// Rename a skill directory. The on-disk layout is `<dir>/SKILL.md`,
/// so the path we move is the directory containing SKILL.md.
fn move_skill_dir(item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope])
    -> Result<RestoreInfo, WardError>
{
    let from_path = skill_dir_from_item(item);
    let to_dir = resolve_skill_dir(dest_scope_id, scopes)
        .ok_or_else(|| WardError::NotFound(format!("Cannot resolve skills dir for {dest_scope_id}")))?;
    let dir_name = from_path.file_name().and_then(|s| s.to_str())
        .ok_or_else(|| WardError::NotFound("Skill dir has no name".into()))?;
    let to_path = to_dir.join(dir_name);
    if to_path.exists() {
        return Err(WardError::NotFound(format!("Skill {dir_name} already exists at destination")));
    }
    std::fs::create_dir_all(&to_dir)?;
    safe_rename(&from_path, &to_path, true)?;
    Ok(RestoreInfo {
        kind: "file".into(),
        original_path: from_path.display().to_string(),
        current_path: Some(to_path.display().to_string()),
        backup_bytes: None,
        mcp_entry: None,
        mcp_key: None,
        mcp_parent_key: None,
        mcp_scope: None,
    })
}

fn skill_dir_from_item(item: &HarnessItem) -> PathBuf {
    let p = PathBuf::from(&item.path);
    // item.path for skills points at SKILL.md; move the parent.
    if p.file_name().and_then(|s| s.to_str()) == Some("SKILL.md") {
        p.parent().map(|x| x.to_path_buf()).unwrap_or(p)
    } else {
        p
    }
}

fn move_mcp(_ctx: &Ctx, item: &HarnessItem, dest_scope_id: &str, scopes: &[Scope])
    -> Result<RestoreInfo, WardError>
{
    let from_json = PathBuf::from(&item.path);
    let to_json = resolve_mcp_json(dest_scope_id, scopes)
        .ok_or_else(|| WardError::NotFound(format!("Cannot resolve .mcp.json for {dest_scope_id}")))?;

    // Identify which parent holds this entry (mcpServers | projects[<key>].mcpServers).
    let parent_key = detect_mcp_parent(&from_json, &item.name, scopes);

    // Read source.
    let mut from_root = read_json_or_empty(&from_json)?;
    let entry = extract_mcp_entry(&mut from_root, &parent_key, &item.name)
        .ok_or_else(|| WardError::NotFound(format!("Server {} not found in {}", item.name, from_json.display())))?;

    // Read destination (creating if missing).
    let mut to_root = read_json_or_empty(&to_json)?;
    ensure_mcp_parent(&mut to_root, &parent_key);
    if to_root.get(parent_key.object_key())
        .and_then(|v| v.get(parent_key.entry_key()))
        .is_some()
    {
        return Err(WardError::NotFound(format!("Server {} already exists in destination", item.name)));
    }
    insert_mcp_entry(&mut to_root, &parent_key, &item.name, entry.clone());

    // Write both files (mcp_dir parent if needed).
    if let Some(parent) = to_json.parent() { std::fs::create_dir_all(parent)?; }
    write_json(&to_json, &to_root)?;
    write_json(&from_json, &from_root)?;

    Ok(RestoreInfo {
        kind: "mcp-entry".into(),
        original_path: from_json.display().to_string(),
        current_path: Some(to_json.display().to_string()),
        backup_bytes: None,
        mcp_entry: Some(entry),
        mcp_key: Some(item.name.clone()),
        mcp_parent_key: Some(parent_key.object_key().to_string()),
        mcp_scope: parent_key.scope_key().map(|s| s.to_string()),
    })
}

// ── Delete implementations ─────────────────────────────────────────────

fn delete_single_file(_ctx: &Ctx, item: &HarnessItem) -> Result<RestoreInfo, WardError> {
    let p = PathBuf::from(&item.path);
    let bytes = std::fs::read(&p)?;
    std::fs::remove_file(&p)?;
    Ok(RestoreInfo {
        kind: "file".into(),
        original_path: p.display().to_string(),
        current_path: None,
        backup_bytes: Some(bytes),
        mcp_entry: None,
        mcp_key: None,
        mcp_parent_key: None,
        mcp_scope: None,
    })
}

fn delete_skill_dir(_ctx: &Ctx, item: &HarnessItem) -> Result<RestoreInfo, WardError> {
    let dir = skill_dir_from_item(item);
    let tree = capture_dir(&dir)?;
    std::fs::remove_dir_all(&dir)?;
    let bytes = serde_json::to_vec(&tree)
        .map_err(|e| WardError::NotFound(format!("serialize skill tree: {e}")))?;
    Ok(RestoreInfo {
        kind: "file".into(),
        original_path: dir.display().to_string(),
        current_path: None,
        backup_bytes: Some(bytes),
        mcp_entry: None,
        mcp_key: None,
        mcp_parent_key: None,
        mcp_scope: None,
    })
}

fn delete_session(_ctx: &Ctx, item: &HarnessItem) -> Result<RestoreInfo, WardError> {
    let p = PathBuf::from(&item.path);
    let bytes = std::fs::read(&p)?;
    std::fs::remove_file(&p)?;
    // Also remove the per-session subagent dir if it exists.
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if !stem.is_empty() {
        let sib = p.with_file_name(stem);
        let _ = std::fs::remove_dir_all(&sib);
    }
    Ok(RestoreInfo {
        kind: "file".into(),
        original_path: p.display().to_string(),
        current_path: None,
        backup_bytes: Some(bytes),
        mcp_entry: None,
        mcp_key: None,
        mcp_parent_key: None,
        mcp_scope: None,
    })
}

fn delete_mcp_entry(_ctx: &Ctx, item: &HarnessItem) -> Result<RestoreInfo, WardError> {
    let from_json = PathBuf::from(&item.path);
    let parent_key = detect_mcp_parent(&from_json, &item.name, &[]);
    let mut root = read_json_or_empty(&from_json)?;
    let entry = extract_mcp_entry(&mut root, &parent_key, &item.name)
        .ok_or_else(|| WardError::NotFound(format!("Server {} not found in {}", item.name, from_json.display())))?;
    write_json(&from_json, &root)?;
    Ok(RestoreInfo {
        kind: "mcp-entry".into(),
        original_path: from_json.display().to_string(),
        current_path: None,
        backup_bytes: None,
        mcp_entry: Some(entry),
        mcp_key: Some(item.name.clone()),
        mcp_parent_key: Some(parent_key.object_key().to_string()),
        mcp_scope: parent_key.scope_key().map(|s| s.to_string()),
    })
}

// ── Restore ────────────────────────────────────────────────────────────

fn restore_file(ctx: &Ctx, info: &RestoreInfo) -> Result<(), WardError> {
    if let Some(bytes) = &info.backup_bytes {
        // Delete case: write bytes back to original_path.
        let abs = ensure_under_home(Path::new(&info.original_path), ctx.home)?;
        if let Some(parent) = abs.parent() { std::fs::create_dir_all(parent)?; }
        // For skill dirs, bytes is a JSON map of {relpath: bytes}.
        if bytes.len() >= 2 && bytes[0] == b'{' {
            if let Ok(tree) = serde_json::from_slice::<SkillTree>(bytes) {
                restore_skill_tree(&abs, &tree)?;
                return Ok(());
            }
        }
        std::fs::write(&abs, bytes)?;
    } else if let Some(cur) = &info.current_path {
        // Move case: rename current_path back to original_path.
        let abs_cur = ensure_under_home(Path::new(cur), ctx.home)?;
        let abs_orig = ensure_under_home(Path::new(&info.original_path), ctx.home)?;
        if abs_cur.exists() {
            if let Some(parent) = abs_orig.parent() { std::fs::create_dir_all(parent)?; }
            // If original_path still exists (race), bail with a clear error.
            if abs_orig.exists() {
                return Err(WardError::NotFound(format!(
                    "Cannot restore: {} already exists", info.original_path
                )));
            }
            safe_rename(&abs_cur, &abs_orig, abs_cur.is_dir())?;
        }
    } else {
        return Err(WardError::NotFound("RestoreInfo has no payload".into()));
    }
    Ok(())
}

fn restore_mcp_entry(ctx: &Ctx, info: &RestoreInfo) -> Result<(), WardError> {
    let json = ensure_under_home(Path::new(&info.original_path), ctx.home)?;
    let mut root = read_json_or_empty(&json)?;
    let entry = info.mcp_entry.clone()
        .ok_or_else(|| WardError::NotFound("RestoreInfo has no mcp_entry".into()))?;
    let key = info.mcp_key.clone()
        .ok_or_else(|| WardError::NotFound("RestoreInfo has no mcp_key".into()))?;
    let pk = info.mcp_parent_key.clone()
        .ok_or_else(|| WardError::NotFound("RestoreInfo has no mcp_parent_key".into()))?;
    let parent_key = McpParentKey::from_parts(&pk, info.mcp_scope.as_deref());
    ensure_mcp_parent(&mut root, &parent_key);
    insert_mcp_entry(&mut root, &parent_key, &key, entry);
    if let Some(parent) = json.parent() { std::fs::create_dir_all(parent)?; }
    write_json(&json, &root)?;
    Ok(())
}

// ── Skill directory tree capture / restore ─────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct SkillTree {
    files: Vec<(String, Vec<u8>)>,
}

fn capture_dir(dir: &Path) -> Result<SkillTree, WardError> {
    let mut files = Vec::new();
    walk_capture(dir, dir, &mut files)?;
    Ok(SkillTree { files })
}

fn walk_capture(root: &Path, cur: &Path, out: &mut Vec<(String, Vec<u8>)>)
    -> Result<(), WardError>
{
    for entry in std::fs::read_dir(cur)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            walk_capture(root, &path, out)?;
        } else if file_type.is_file() {
            let rel = path.strip_prefix(root)
                .map_err(|e| WardError::NotFound(format!("relpath: {e}")))?;
            let rel_str = rel.to_string_lossy().to_string();
            let bytes = std::fs::read(&path)?;
            out.push((rel_str, bytes));
        }
    }
    Ok(())
}

fn restore_skill_tree(dir: &Path, tree: &SkillTree) -> Result<(), WardError> {
    std::fs::create_dir_all(dir)?;
    for (rel, bytes) in &tree.files {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() { std::fs::create_dir_all(parent)?; }
        std::fs::write(&p, bytes)?;
    }
    Ok(())
}

// ── JSON MCP helpers ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct McpParentKey {
    /// "mcpServers" or "projects"
    object: String,
    /// Present only when `object == "projects"` — the project key inside
    /// `projects` under which the entry lives.
    project: Option<String>,
}

impl McpParentKey {
    fn mcp_servers() -> Self { Self { object: "mcpServers".into(), project: None } }

    fn projects(project: impl Into<String>) -> Self {
        Self { object: "projects".into(), project: Some(project.into()) }
    }

    fn object_key(&self) -> &str { &self.object }

    fn entry_key(&self) -> &str { "mcpServers" }

    fn scope_key(&self) -> Option<&str> { self.project.as_deref() }

    fn from_parts(object: &str, project: Option<&str>) -> Self {
        Self {
            object: object.to_string(),
            project: project.map(|s| s.to_string()),
        }
    }
}

/// Detect which parent holds `server_name` inside `json_path`. Looks
/// for `mcpServers[server_name]` first, then for any project entry
/// matching the JSON file's own location (only relevant for
/// `~/.claude.json`).
fn detect_mcp_parent(json_path: &Path, server_name: &str, scopes: &[Scope]) -> McpParentKey {
    if let Ok(content) = std::fs::read_to_string(json_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(servers) = v.get("mcpServers").and_then(|x| x.as_object()) {
                if servers.contains_key(server_name) {
                    return McpParentKey::mcp_servers();
                }
            }
            // Search projects[<*>].mcpServers.
            if let Some(projs) = v.get("projects").and_then(|x| x.as_object()) {
                for (key, proj) in projs {
                    if let Some(servers) = proj.get("mcpServers").and_then(|x| x.as_object()) {
                        if servers.contains_key(server_name) {
                            return McpParentKey::projects(key.clone());
                        }
                    }
                }
            }
        }
    }
    let _ = scopes;
    // Default to mcpServers — move/delete paths will surface a clear
    // "server not found" error if the parent key is wrong.
    McpParentKey::mcp_servers()
}

fn read_json_or_empty(path: &Path) -> Result<serde_json::Value, WardError> {
    match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| WardError::NotFound(format!("parse {}: {e}", path.display()))),
        Err(_) => Ok(serde_json::json!({})),
    }
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), WardError> {
    let s = serde_json::to_string_pretty(value)
        .map_err(|e| WardError::NotFound(format!("serialize: {e}")))?;
    std::fs::write(path, format!("{s}\n"))?;
    Ok(())
}

fn ensure_mcp_parent(root: &mut serde_json::Value, parent: &McpParentKey) {
    if !root.is_object() { *root = serde_json::json!({}); }
    let obj = root.as_object_mut().unwrap();
    if parent.object == "mcpServers" {
        if !obj.contains_key("mcpServers") {
            obj.insert("mcpServers".into(), serde_json::json!({}));
        }
    } else if parent.object == "projects" {
        if !obj.contains_key("projects") {
            obj.insert("projects".into(), serde_json::json!({}));
        }
        let projs = obj.get_mut("projects").unwrap().as_object_mut().unwrap();
        let key = parent.project.clone().unwrap_or_default();
        if !projs.contains_key(&key) {
            projs.insert(key.clone(), serde_json::json!({}));
        }
        let proj = projs.get_mut(&key).unwrap().as_object_mut().unwrap();
        if !proj.contains_key("mcpServers") {
            proj.insert("mcpServers".into(), serde_json::json!({}));
        }
    }
}

fn insert_mcp_entry(root: &mut serde_json::Value, parent: &McpParentKey,
                    key: &str, entry: serde_json::Value)
{
    if parent.object == "mcpServers" {
        let obj = root.as_object_mut().unwrap()
            .get_mut("mcpServers").unwrap()
            .as_object_mut().unwrap();
        obj.insert(key.into(), entry);
    } else if parent.object == "projects" {
        let projs = root.as_object_mut().unwrap()
            .get_mut("projects").unwrap()
            .as_object_mut().unwrap();
        let proj_key = parent.project.clone().unwrap_or_default();
        let proj = projs.get_mut(&proj_key).unwrap()
            .as_object_mut().unwrap()
            .get_mut("mcpServers").unwrap()
            .as_object_mut().unwrap();
        proj.insert(key.into(), entry);
    }
}

fn extract_mcp_entry(root: &mut serde_json::Value, parent: &McpParentKey, key: &str)
    -> Option<serde_json::Value>
{
    if parent.object == "mcpServers" {
        let obj = root.as_object_mut()?.get_mut("mcpServers")?.as_object_mut()?;
        obj.remove(key)
    } else if parent.object == "projects" {
        let projs = root.as_object_mut()?.get_mut("projects")?.as_object_mut()?;
        let proj_key = parent.project.clone()?;
        let proj = projs.get_mut(&proj_key)?.as_object_mut()?;
        let servers = proj.get_mut("mcpServers")?.as_object_mut()?;
        servers.remove(key)
    } else {
        None
    }
}

// ── safeRename (CCO parity) ────────────────────────────────────────────

fn safe_rename(from: &Path, to: &Path, is_dir: bool) -> Result<(), WardError> {
    match std::fs::rename(from, to) {
        Ok(_) => Ok(()),
        Err(e) if e.raw_os_error() == Some(18) /* EXDEV */ => {
            if is_dir {
                copy_dir_recursive(from, to)?;
                std::fs::remove_dir_all(from)?;
            } else {
                std::fs::copy(from, to)?;
                std::fs::remove_file(from)?;
            }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), WardError> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let from_child = entry.path();
        let to_child = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from_child, &to_child)?;
        } else {
            std::fs::copy(&from_child, &to_child)?;
        }
    }
    Ok(())
}

// ════════════════════════════════════════════════════════════════════════
// TESTS — ported from CCO `tests/unit/test-move-destinations.mjs`.
// Each test asserts parity with the JS golden.
// ════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    const HOME_FOR_TEST: &str = "/Users/testhome";

    fn test_scopes() -> Vec<Scope> {
        vec![
            Scope { id: "global".into(), kind: "global".into(),
                label: "Global".into(), root: "/Users/testhome/.claude".into() },
            Scope { id: "-proj-a".into(), kind: "project".into(),
                label: "project-a".into(), root: "/tmp/project-a".into() },
            Scope { id: "-proj-b".into(), kind: "project".into(),
                label: "project-b".into(), root: "/tmp/project-b".into() },
            Scope { id: "-home".into(), kind: "project".into(),
                label: "home".into(), root: HOME_FOR_TEST.into() },
        ]
    }

    fn make_item(category: &str, scope_id: &str) -> HarnessItem {
        HarnessItem {
            category: category.into(),
            scope_id: scope_id.into(),
            name: "test-item".into(),
            description: String::new(),
            path: "/fake/test-item".into(),
            movable: true,
            deletable: true,
            locked: false,
            effective: None,
        }
    }

    fn dest_ids(item: &HarnessItem) -> Vec<String> {
        get_valid_destinations(Path::new(HOME_FOR_TEST), item, &test_scopes())
            .into_iter().map(|d| d.scope_id).collect()
    }

    // ── Movable categories ──

    #[test]
    fn skill_can_move_to_global_and_project_scopes_not_home_overlap() {
        let item = make_item("skill", "-proj-a");
        let dests = dest_ids(&item);
        assert!(dests.contains(&"global".into()), "global should be a destination");
        assert!(dests.contains(&"-proj-b".into()), "other project should be a destination");
        assert!(!dests.contains(&"-proj-a".into()), "current scope should NOT be a destination");
        assert!(!dests.contains(&"-home".into()), "home scope should NOT be a destination (overlaps global .claude)");
    }

    #[test]
    fn memory_can_move_to_global_and_project_scopes() {
        let item = make_item("memory", "global");
        let dests = dest_ids(&item);
        assert!(dests.contains(&"-proj-a".into()), "project-a should be a destination");
        assert!(dests.contains(&"-proj-b".into()), "project-b should be a destination");
        assert!(!dests.contains(&"global".into()), "current scope (global) should NOT be a destination");
        assert!(!dests.contains(&"-home".into()), "home scope should NOT be a destination");
    }

    #[test]
    fn command_can_move_to_global_and_project_scopes() {
        let item = make_item("command", "-proj-a");
        let dests = dest_ids(&item);
        assert!(dests.contains(&"global".into()));
        assert!(dests.contains(&"-proj-b".into()));
        assert!(!dests.contains(&"-proj-a".into()));
        assert!(!dests.contains(&"-home".into()));
    }

    #[test]
    fn agent_can_move_to_global_and_project_scopes() {
        let item = make_item("agent", "global");
        let dests = dest_ids(&item);
        assert!(dests.contains(&"-proj-a".into()));
        assert!(dests.contains(&"-proj-b".into()));
        assert!(!dests.contains(&"global".into()));
        assert!(!dests.contains(&"-home".into()));
    }

    #[test]
    fn mcp_can_move_to_any_scope_including_home_overlap() {
        let item = make_item("mcp", "-proj-a");
        let dests = dest_ids(&item);
        assert!(dests.contains(&"global".into()), "global should be a destination");
        assert!(dests.contains(&"-proj-b".into()), "other project should be a destination");
        assert!(dests.contains(&"-home".into()), "home scope IS valid for MCP (uses claudeProjectDir)");
        assert!(!dests.contains(&"-proj-a".into()), "current scope should NOT be a destination");
    }

    // ── Locked categories ──

    #[test]
    fn plan_returns_empty_destinations() {
        let item = make_item("plan", "-proj-a");
        assert_eq!(dest_ids(&item), Vec::<String>::new());
    }

    #[test]
    fn rule_returns_empty_destinations() {
        let item = make_item("rule", "-proj-a");
        assert_eq!(dest_ids(&item), Vec::<String>::new());
    }

    #[test]
    fn config_returns_empty_destinations() {
        let item = make_item("config", "-proj-a");
        assert_eq!(dest_ids(&item), Vec::<String>::new());
    }

    #[test]
    fn hook_returns_empty_destinations() {
        let item = make_item("hook", "global");
        assert_eq!(dest_ids(&item), Vec::<String>::new());
    }

    #[test]
    fn plugin_returns_empty_destinations() {
        let item = make_item("plugin", "global");
        assert_eq!(dest_ids(&item), Vec::<String>::new());
    }

    #[test]
    fn session_returns_empty_destinations() {
        let item = make_item("session", "-proj-a");
        assert_eq!(dest_ids(&item), Vec::<String>::new());
    }

    // ── Locked items ──

    #[test]
    fn locked_item_always_returns_empty_regardless_of_category() {
        let mut item = make_item("skill", "-proj-a");
        item.locked = true;
        assert_eq!(dest_ids(&item), Vec::<String>::new());
    }

    #[test]
    fn every_non_movable_category_returns_empty_when_unlocked() {
        for cat in ["plan", "rule", "config", "hook", "plugin", "session"] {
            let item = make_item(cat, "-proj-a");
            assert_eq!(dest_ids(&item), Vec::<String>::new(), "{cat} should have no destinations");
        }
    }

    #[test]
    fn every_movable_category_returns_non_empty_when_unlocked() {
        for cat in ["skill", "memory", "command", "agent", "mcp"] {
            let item = make_item(cat, "-proj-a");
            assert!(!dest_ids(&item).is_empty(), "{cat} should have destinations");
        }
    }

    // ── Scope-resolver parity tests ──

    #[test]
    fn shares_global_claude_dir_true_for_home_scope() {
        let s = Scope { id: "-home".into(), kind: "project".into(),
            label: "home".into(), root: HOME_FOR_TEST.into() };
        assert!(shares_global_claude_dir(Path::new(HOME_FOR_TEST), &s));
    }

    #[test]
    fn shares_global_claude_dir_false_for_normal_project() {
        let s = Scope { id: "-proj-a".into(), kind: "project".into(),
            label: "a".into(), root: "/tmp/project-a".into() };
        assert!(!shares_global_claude_dir(Path::new(HOME_FOR_TEST), &s));
    }

    #[test]
    fn resolve_skill_dir_global_is_claude_skills() {
        let scopes = test_scopes();
        let p = resolve_skill_dir("global", &scopes).unwrap();
        assert!(p.ends_with(".claude/skills"), "got {p:?}");
    }

    #[test]
    fn resolve_skill_dir_project_is_repo_claude_skills() {
        let scopes = test_scopes();
        let p = resolve_skill_dir("-proj-a", &scopes).unwrap();
        assert!(p.ends_with("/tmp/project-a/.claude/skills"), "got {p:?}");
    }

    #[test]
    fn resolve_mcp_json_project_is_repo_mcp_json() {
        let scopes = test_scopes();
        let p = resolve_mcp_json("-proj-a", &scopes).unwrap();
        assert!(p.ends_with("/tmp/project-a/.mcp.json"), "got {p:?}");
    }

    #[test]
    fn validate_move_rejects_locked() {
        let mut item = make_item("skill", "-proj-a");
        item.locked = true;
        let scopes = test_scopes();
        assert!(validate_move(&item, "-proj-b", &scopes).is_err());
    }

    #[test]
    fn validate_move_rejects_same_scope() {
        let item = make_item("skill", "-proj-a");
        let scopes = test_scopes();
        assert!(validate_move(&item, "-proj-a", &scopes).is_err());
    }

    #[test]
    fn validate_move_rejects_non_movable_category() {
        // plan and rule ARE in CCO's movable list (move/rename work) but
        // `getValidDestinations` returns empty for them, so the UI never
        // surfaces a destination. A truly non-movable category
        // (config/hook/plugin/session/setting) is rejected here.
        let item = make_item("config", "-proj-a");
        let scopes = test_scopes();
        assert!(validate_move(&item, "-proj-b", &scopes).is_err());
    }

    #[test]
    fn validate_move_accepts_plan_and_rule_even_though_destinations_empty() {
        let plan = make_item("plan", "-proj-a");
        let rule = make_item("rule", "-proj-a");
        let scopes = test_scopes();
        assert!(validate_move(&plan, "-proj-b", &scopes).is_ok());
        assert!(validate_move(&rule, "-proj-b", &scopes).is_ok());
    }

    #[test]
    fn validate_move_rejects_unknown_scope() {
        let item = make_item("skill", "-proj-a");
        let scopes = test_scopes();
        assert!(validate_move(&item, "-does-not-exist", &scopes).is_err());
    }

    #[test]
    fn validate_move_accepts_valid_pair() {
        let item = make_item("skill", "-proj-a");
        let scopes = test_scopes();
        assert!(validate_move(&item, "-proj-b", &scopes).is_ok());
    }
}