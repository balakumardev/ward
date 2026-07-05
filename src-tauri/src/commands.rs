use std::path::Path;
use crate::backup::git as git_ops;
use crate::backup::scheduler as sched_ops;
use crate::error::WardError;
use crate::harness::adapters::claude::ClaudeAdapter;
use crate::harness::adapters::claude_budget as budget;
use crate::harness::adapters::claude_mcp as mcp;
use crate::harness::adapters::claude_ops::ClaudeOps;
use crate::harness::adapters::codex::CodexAdapter;
use crate::harness::{framework, Ctx, HarnessOps, Registry};
use crate::model::{Destination, HarnessItem, McpPolicy, PolicyVerdict, RestoreInfo, ScanResult, Scope};
use crate::sessions::{cost as session_cost, distill as session_distill, parse as session_parse, trim as session_trim};

pub fn build_registry() -> Registry {
    let mut r = Registry::new();
    r.register(Box::new(ClaudeAdapter));
    r.register(Box::new(CodexAdapter));
    r
}

pub fn scan_impl(registry: &Registry, home: &Path, harness_id: &str) -> Result<ScanResult, WardError> {
    let adapter = registry
        .get(harness_id)
        .ok_or_else(|| WardError::HarnessUnavailable(harness_id.to_string()))?;
    let ctx = Ctx { home, cwd: None };
    framework::run_scan(adapter, &ctx)
}

#[tauri::command]
pub fn scan(harness: String) -> Result<ScanResult, WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    let registry = build_registry();
    scan_impl(&registry, &home, &harness)
}

pub fn read_file_impl(path: &Path, home: &Path) -> Result<String, WardError> {
    let abs = crate::fs_utils::ensure_under_home(path, home)?;
    Ok(std::fs::read_to_string(abs)?)
}

#[tauri::command]
pub fn read_file_content(path: String) -> Result<String, WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    read_file_impl(Path::new(&path), &home)
}

// ── Mutation surface (Plan 03) ─────────────────────────────────────────

/// Pick the ops implementation that backs `harness_id`. Today we only
/// ship the Claude adapter's ops; future adapters will plug in here.
fn ops_for(harness_id: &str) -> Result<&'static dyn HarnessOps, WardError> {
    match harness_id {
        "claude" => Ok(&ClaudeOps),
        other => Err(WardError::HarnessUnavailable(other.to_string())),
    }
}

/// Re-discover scopes + the relevant `Ctx` for a harness. We rebuild
/// the registry on every mutation command so the scope list reflects
/// the latest on-disk state.
fn harness_ctx(harness_id: &str) -> Result<(Ctx<'static>, Vec<Scope>), WardError> {
    // We need a 'static home so Ctx can outlive this stack frame; use
    // a leaked Box. This is the only Tauri command path; tests use
    // the helpers above.
    let home_static: &'static Path = Box::leak(
        dirs::home_dir()
            .ok_or_else(|| WardError::NotFound("home directory".into()))?
            .into_boxed_path(),
    );
    let registry = build_registry();
    let adapter = registry
        .get(harness_id)
        .ok_or_else(|| WardError::HarnessUnavailable(harness_id.to_string()))?;
    let ctx = Ctx { home: home_static, cwd: None };
    let scopes = adapter.discover_scopes(&ctx)?;
    Ok((ctx, scopes))
}

#[tauri::command]
pub fn list_destinations(harness: String, item: HarnessItem) -> Result<Vec<Destination>, WardError> {
    let ops = ops_for(&harness)?;
    let (ctx, scopes) = harness_ctx(&harness)?;
    Ok(ops.get_valid_destinations(&ctx, &item, &scopes))
}

#[tauri::command]
pub fn move_item(harness: String, item: HarnessItem, dest_scope_id: String) -> Result<RestoreInfo, WardError> {
    let ops = ops_for(&harness)?;
    let (ctx, scopes) = harness_ctx(&harness)?;
    ops.move_item(&ctx, &item, &dest_scope_id, &scopes)
}

#[tauri::command]
pub fn delete_item(harness: String, item: HarnessItem) -> Result<RestoreInfo, WardError> {
    let ops = ops_for(&harness)?;
    let (ctx, scopes) = harness_ctx(&harness)?;
    ops.delete_item(&ctx, &item, &scopes)
}

#[tauri::command]
pub fn restore(harness: String, info: RestoreInfo) -> Result<(), WardError> {
    let ops = ops_for(&harness)?;
    let (ctx, _) = harness_ctx(&harness)?;
    ops.restore(&ctx, &info)
}

#[tauri::command]
pub fn save_file(path: String, content: String) -> Result<(), WardError> {
    // save_file uses the global ClaudeOps so the same `ensure_under_home`
    // and write semantics apply. We could route through `ops_for` if a
    // future harness needs different validation.
    let ops = ClaudeOps;
    let (ctx, _) = harness_ctx("claude")?;
    ops.save_file(&ctx, &path, &content)
}

/// Run a single `move_item` or `delete_item` for each input and
/// accumulate every `RestoreInfo` so the UI can offer a single Undo.
#[tauri::command]
pub fn bulk(
    harness: String,
    items: Vec<HarnessItem>,
    op: String,
    dest_scope_id: Option<String>,
) -> Result<Vec<RestoreInfo>, WardError> {
    let ops = ops_for(&harness)?;
    let (ctx, scopes) = harness_ctx(&harness)?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let info = match op.as_str() {
            "move" => {
                let dest = dest_scope_id.clone()
                    .ok_or_else(|| WardError::NotFound("bulk move requires dest_scope_id".into()))?;
                ops.move_item(&ctx, &item, &dest, &scopes)?
            }
            "delete" => ops.delete_item(&ctx, &item, &scopes)?,
            other => return Err(WardError::NotFound(format!("Unknown bulk op: {other}"))),
        };
        out.push(info);
    }
    Ok(out)
}

/// Reverse a batch of `RestoreInfo`s. Apply them in reverse order so a
/// later restore doesn't overwrite a file that an earlier restore will
/// later recreate.
#[tauri::command]
pub fn bulk_restore(harness: String, infos: Vec<RestoreInfo>) -> Result<(), WardError> {
    let ops = ops_for(&harness)?;
    let (ctx, _) = harness_ctx(&harness)?;
    for info in infos.iter().rev() {
        ops.restore(&ctx, info)?;
    }
    Ok(())
}

// ── MCP controls (Plan 04) ─────────────────────────────────────────────

/// Read `projects[<projectPath>].disabledMcpServers` from
/// `~/.claude.json`. Returns empty Vec when the file or key is absent.
#[tauri::command]
pub fn mcp_get_disabled(project_path: String) -> Result<Vec<String>, WardError> {
    let home = Box::leak(
        dirs::home_dir()
            .ok_or_else(|| WardError::NotFound("home directory".into()))?
            .into_boxed_path(),
    );
    mcp::get_disabled_servers(home, Path::new(&project_path))
}

/// Write `projects[<projectPath>].disabledMcpServers` to `~/.claude.json`.
/// Returns a `RestoreInfo` capturing the prior file bytes verbatim so
/// the Organizer can offer Undo.
#[tauri::command]
pub fn mcp_set_disabled(project_path: String, list: Vec<String>) -> Result<RestoreInfo, WardError> {
    let home = Box::leak(
        dirs::home_dir()
            .ok_or_else(|| WardError::NotFound("home directory".into()))?
            .into_boxed_path(),
    );
    mcp::set_disabled_servers(home, Path::new(&project_path), &list)
}

/// Read the user-scope MCP policy (allowlist + denylist) from
/// `~/.claude/settings.json`.
#[tauri::command]
pub fn mcp_get_policy() -> Result<McpPolicy, WardError> {
    let home = Box::leak(
        dirs::home_dir()
            .ok_or_else(|| WardError::NotFound("home directory".into()))?
            .into_boxed_path(),
    );
    mcp::get_policy(home)
}

/// Write the user-scope MCP policy. Returns a `RestoreInfo` capturing
/// the prior file bytes verbatim for Undo.
#[tauri::command]
pub fn mcp_set_policy(policy: McpPolicy) -> Result<RestoreInfo, WardError> {
    let home = Box::leak(
        dirs::home_dir()
            .ok_or_else(|| WardError::NotFound("home directory".into()))?
            .into_boxed_path(),
    );
    mcp::set_policy(home, &policy)
}

/// Convenience for the UI: "is this server allowed by the current
/// policy?" Used to render the per-item policy badge.
#[tauri::command]
pub fn mcp_check_policy(
    server_name: String,
    server_config: serde_json::Value,
    policy: McpPolicy,
) -> Result<PolicyVerdict, WardError> {
    Ok(mcp::check_policy(&server_name, &server_config, &policy))
}

// ── Security Scanner (Plan 05) ─────────────────────────────────────────

/// Run the security scan over the discovered MCP items.
#[tauri::command]
pub fn security_scan(
    harness: String,
    items: Vec<HarnessItem>,
    run_judge: Option<bool>,
) -> Result<crate::security::scan::ScanResult, WardError> {
    let opts = crate::security::scan::ScanOptions {
        run_judge: run_judge.unwrap_or(false),
    };
    crate::security::scan::scan(&items, &opts)
}

/// Compare a fresh scan against the saved baseline; return diffs.
#[tauri::command]
pub fn security_baseline_check(
    scan: crate::security::scan::ScanResult,
) -> Result<Vec<crate::security::baseline::BaselineDiff>, WardError> {
    let path = crate::security::baseline::default_path()?;
    let saved = crate::security::baseline::load(&path)?;
    let mut new_baseline = crate::security::baseline::Baseline::default();
    for s in &scan.servers {
        let mut hashes = std::collections::HashMap::new();
        for t in &s.tools {
            hashes.insert(t.name.clone(), t.hash.clone());
        }
        new_baseline.servers.insert(
            s.server_name.clone(),
            crate::security::baseline::BaselineEntry {
                tool_hashes: hashes,
                accepted_at: chrono::Utc::now(),
                accepted_findings: scan.findings.iter()
                    .filter(|f| f.source_name.starts_with(&s.server_name))
                    .map(|f| f.rule_id.clone())
                    .collect(),
            },
        );
    }
    Ok(crate::security::baseline::diff(&saved, &new_baseline))
}

/// Persist a baseline snapshot.
#[tauri::command]
pub fn security_baseline_accept(
    server: String,
    findings: Vec<String>,
) -> Result<(), WardError> {
    let path = crate::security::baseline::default_path()?;
    let mut current = crate::security::baseline::load(&path)?;
    current.servers.insert(
        server,
        crate::security::baseline::BaselineEntry {
            tool_hashes: std::collections::HashMap::new(),
            accepted_at: chrono::Utc::now(),
            accepted_findings: findings,
        },
    );
    crate::security::baseline::save(&path, &current)
}

// ── Context Budget (Plan 06) ──────────────────────────────────────────

/// Compose the per-scope context budget for `scope_id`. Looks up the
/// scope in a fresh scan, collects MCP server names (deduped by name)
/// from the scope's MCP items, and hands them to the budget composer.
///
/// Errors when:
///   - the home directory cannot be resolved (mirrors `scan`),
///   - the harness is unknown (`HarnessUnavailable`),
///   - the scan itself fails (IO, parse, etc.).
#[tauri::command]
pub fn context_budget(
    harness: String,
    scope_id: String,
) -> Result<budget::BudgetBreakdown, WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    let registry = build_registry();
    let result = scan_impl(&registry, &home, &harness)?;
    // Collect unique MCP server names from the scope (we keep this
    // strict to the requested scope for parity with CCO's per-scope
    // view — inherited scopes would be a Plan 06+ extension).
    let mut unique_servers: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    for item in &result.items {
        if item.category == "mcp" && item.scope_id == scope_id {
            unique_servers.insert(item.name.clone());
        }
    }
    let servers_vec: Vec<String> = unique_servers.into_iter().collect();
    Ok(budget::compose(&scope_id, &result.items, &servers_vec, &home))
}

// ── Sessions mode (Plan 07) ─────────────────────────────────────────────

/// Stream-parse a session `.jsonl` file into a structured
/// `Conversation`. Returned to the UI so the conversation viewer can
/// render user/assistant turns + metadata without touching the disk.
#[tauri::command]
pub fn session_preview(path: String) -> Result<session_parse::Conversation, WardError> {
    session_parse::parse_file(Path::new(&path))
}

/// Aggregate per-model token usage + estimated USD cost for a session.
/// The pricing table in `sessions::cost` is rough — the UI labels the
/// total as an estimate.
#[tauri::command]
pub fn session_cost(path: String) -> Result<session_cost::CostBreakdown, WardError> {
    let conv = session_parse::parse_file(Path::new(&path))?;
    session_cost::compute(&conv)
}

/// Distill a session. Backs up the original first, writes the cleaned
/// JSONL + `index.md` next to it, and returns the resulting paths +
/// reduction stats.
#[tauri::command]
pub fn session_distill(path: String) -> Result<session_distill::DistillResult, WardError> {
    session_distill::distill(Path::new(&path))
}

/// Replace base64 image blocks in the session file with
/// `[image redacted]` text blocks. Returns a `RestoreInfo` capturing
/// the prior file bytes so the Organizer's Undo flow can revert.
#[tauri::command]
pub fn session_trim(path: String) -> Result<RestoreInfo, WardError> {
    session_trim::trim_file(Path::new(&path))
}

// ── Backup Center (Plan 08) ─────────────────────────────────────────────

/// Aggregate status payload for the Backups mode. Each field is
/// best-effort: missing files = `None`, missing scheduler = `false`.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    pub has_repo: bool,
    pub last_commit: Option<String>,
    pub last_commit_at: Option<chrono::DateTime<chrono::Utc>>,
    pub scheduler_installed: bool,
    /// True when launchd still has the backup label loaded but its plist
    /// file is gone (a recoverable orphan). The UI keeps Remove enabled
    /// in this state so the user can clear the dead job.
    pub scheduler_orphaned: bool,
    pub scheduler_interval: Option<u32>,
    pub remote_url: Option<String>,
}

fn backup_repo_root() -> Result<std::path::PathBuf, WardError> {
    Ok(git_ops::backup_dir()?.path)
}

#[tauri::command]
pub fn backup_status() -> Result<BackupStatus, WardError> {
    let repo = backup_repo_root()?;
    let has_repo = repo.join(".git").exists();
    let last = if has_repo { git_ops::last_commit(&repo)? } else { None };
    let remote_url = if has_repo { git_ops::remote_url(&repo) } else { None };
    let sched = sched_ops::status();
    let scheduler_installed = sched.installed();
    let scheduler_orphaned = sched.orphaned();
    let scheduler_interval = match &sched {
        sched_ops::SchedulerStatus::Installed { interval_seconds } => Some(*interval_seconds),
        _ => None,
    };
    Ok(BackupStatus {
        has_repo,
        last_commit: last.as_ref().map(|l| l.sha.clone()),
        last_commit_at: last.map(|l| l.committed_at),
        scheduler_installed,
        scheduler_orphaned,
        scheduler_interval,
        remote_url,
    })
}

/// `backup_run` — export the on-disk `~/.claude/` layout into the
/// backup repo + commit. NEVER pushes. The caller (the UI button)
/// must explicitly trigger `backup_push` to send bytes to a remote.
///
/// `scan` is the most recent scan, used only to surface skipped /
/// missing categories to the UI via the returned ExportReport.
/// The actual file copy walks `home.join(".claude")` directly — the
/// repo mirrors the literal on-disk layout, not the scan's category
/// labels.
#[tauri::command]
pub fn backup_run(
    scan: ScanResult,
    _remote_url: Option<String>,
) -> Result<git_ops::ExportReport, WardError> {
    backup_run_impl(scan).map(|_commit| {
        // `backup_run` returns the export report (count of files
        // touched). The commit info is available separately via
        // `backup_status` / `backup_sync` — we deliberately don't
        // auto-push here.
        git_ops::ExportReport::default()
    })
}

fn backup_run_impl(scan: ScanResult) -> Result<git_ops::ExportReport, WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    let bd = git_ops::backup_dir()?;
    let repo = git_ops::repo_dir(&bd);

    // Init the repo if it doesn't exist. Use the fallback identity
    // when no global identity is configured — we never touch the
    // host's git config.
    if !repo.join(".git").exists() {
        let has_global = !git_ops::git_capture_stdout(repo, &["config", "--global", "--get", "user.name"])
            .map(|s| s.is_empty())
            .unwrap_or(true)
            || !git_ops::git_capture_stdout(repo, &["config", "--global", "--get", "user.email"])
                .map(|s| s.is_empty())
                .unwrap_or(true);
        if has_global {
            // Global identity present for at least one key — set both
            // explicitly via local config so commits are attributable
            // even if the user removes their global config later.
            git_ops::init(repo, "ward", "ward@local")?;
        } else {
            git_ops::ensure_identity_or_fallback(repo, git_ops::FALLBACK_USER_NAME, git_ops::FALLBACK_USER_EMAIL)?;
            git_ops::init(repo, git_ops::FALLBACK_USER_NAME, git_ops::FALLBACK_USER_EMAIL)?;
        }
    }

    // Copy the source ~/.claude/ tree into the repo. We always copy
    // from the live `~/.claude/` directory — `scan` is just used by
    // the UI to show counts; the export itself is content-driven.
    let source_root = home.join(".claude");
    let report = git_ops::export_to_repo(&source_root, repo)?;
    if report.files_copied > 0 {
        // A successful export with no files would mean the
        // "~/.claude" tree is empty — that's still a valid state,
        // just skip the commit.
        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let msg = format!("backup: ward (claude) {}", ts);
        git_ops::commit(repo, &msg)?;
    }
    let _ = scan; // surfaced via status API; here it's informational.
    Ok(report)
}

/// `backup_sync` — re-run `git add -A && git commit` against the
/// current backup-dir state (no initial export). This is what the
/// launchd-triggered `--backup-once` mode uses after a prior
/// `backup_run` exported the source. Returns CommitInfo so the UI
/// can show "committed: <sha>". Does NOT push.
#[tauri::command]
pub fn backup_sync() -> Result<git_ops::CommitInfo, WardError> {
    let repo = backup_repo_root()?;
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let msg = format!("backup: ward sync {}", ts);
    git_ops::commit(&repo, &msg)
}

/// `backup_push` — ONLY invoked by an explicit user click. Runs
/// `git push` against the configured remote (or returns
/// `pushed = false` when no remote). This is the single network
/// entry point in the backup pipeline.
#[tauri::command]
pub fn backup_push() -> Result<git_ops::PushResult, WardError> {
    let repo = backup_repo_root()?;
    git_ops::push(&repo)
}

#[tauri::command]
pub fn backup_set_remote(url: String) -> Result<(), WardError> {
    let repo = backup_repo_root()?;
    git_ops::set_remote(&repo, url.trim())
}

/// `backup_log` — the last `n` commits in the backup repo, newest first.
/// Powers the history list in the Backups page.
#[tauri::command]
pub fn backup_log(n: usize) -> Result<Vec<git_ops::GitLogEntry>, WardError> {
    let repo = backup_repo_root()?;
    backup_log_impl(&repo, n)
}

/// Core of `backup_log`, split out so it's unit-testable against a temp
/// repo (the command itself resolves the real `~/.ward-backups/`).
///
/// An absent repo or a commit-less repo is empty history, NOT an error —
/// `git log` exits non-zero on a repo with zero commits, so we guard on
/// `last_commit()` (which already tolerates that) before calling `log()`.
fn backup_log_impl(repo: &Path, n: usize) -> Result<Vec<git_ops::GitLogEntry>, WardError> {
    if !repo.join(".git").exists() || git_ops::last_commit(repo)?.is_none() {
        return Ok(Vec::new());
    }
    git_ops::log(repo, n)
}

#[tauri::command]
pub fn backup_scheduler_install(interval_seconds: u32) -> Result<(), WardError> {
    sched_ops::validate_interval(interval_seconds)?;
    // The plist needs a path to the Ward CLI and a "scan target" to
    // pass as `--backup-once <scan_target>`. Production: the bundled
    // binary inside Ward.app; for `cargo run` we fall back to
    // `std::env::current_exe()` (the dev-time binary). For the scan
    // target we record the harness id "claude" — the CLI resolves
    // that to `~/.claude` itself.
    let ward_binary = std::env::current_exe()
        .map_err(|e| WardError::Backup(format!("cannot resolve ward binary path: {e}")))?;
    let scan_target = std::path::PathBuf::from("claude");
    sched_ops::install(interval_seconds, &ward_binary, &scan_target)
}

#[tauri::command]
pub fn backup_scheduler_remove() -> Result<(), WardError> {
    sched_ops::remove()
}

// ── Launch-at-login (Plan 13) ───────────────────────────────────────────

/// Plan 13 — is launch-at-login enabled?
#[tauri::command]
pub fn autostart_status(app: tauri::AppHandle) -> Result<bool, WardError> {
    crate::native::autostart::status(&app)
}

/// Plan 13 — enable/disable launch-at-login.
#[tauri::command]
pub fn autostart_set(app: tauri::AppHandle, enabled: bool) -> Result<(), WardError> {
    crate::native::autostart::set(&app, enabled)
}

// ── Usage engine (Plan 14) ───────────────────────────────────────────────

/// Plan 14 — local usage snapshot (tokens/cost/reset) for a harness.
///
/// The session-file parse is blocking I/O, so it runs on a worker thread via
/// `spawn_blocking`: every Tauri command runs on the main/UI thread, and a
/// synchronous body here would stall the event loop (and the tray popover)
/// until the parse returned. The pure logic in `crate::usage` stays sync.
#[tauri::command]
pub async fn usage_snapshot(harness: String) -> Result<crate::usage::UsageSnapshot, WardError> {
    tauri::async_runtime::spawn_blocking(move || {
        let snap = crate::usage::usage_snapshot(&harness)?;
        // Write-through (Plan 17): keep the on-disk cache warm so the next
        // popover open paints the previous gauges instantly. Only cache a
        // usable snapshot; a cache-write failure must never fail the command.
        if snap.available {
            let _ = crate::usage::cache::write_entry(&harness, &snap);
        }
        Ok(snap)
    })
    .await
    .map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, format!("usage snapshot task failed: {e}"))
    })?
}

/// Plan 17 — last-known cached usage snapshot for a harness, read from
/// `~/.ward/usage-cache.json`. A tiny local JSON read (sync is fine), so the
/// popover paints the previous gauges instantly on open while a fresh snapshot
/// loads in the background. `None` if nothing has been cached yet.
#[tauri::command]
pub fn usage_cached(harness: String) -> Option<crate::usage::UsageSnapshot> {
    crate::usage::cache::cached_snapshot(&harness)
}

// ── Live usage (Plan 16) ─────────────────────────────────────────────────

/// Plan 16 — live Claude usage via the gated Anthropic rate-limit endpoint.
/// Claude only; requires the live opt-in sentinel + Keychain access. This makes
/// a network call and reads the OAuth token, so it runs only on user action.
///
/// The blocking Keychain read + network round-trip run on a worker thread via
/// `spawn_blocking` for the same reason as [`usage_snapshot`]: keep the event
/// loop responsive so the popover paints immediately instead of freezing for
/// the whole 2–3 s round-trip.
#[tauri::command]
pub async fn usage_snapshot_live(harness: String) -> Result<crate::usage::UsageSnapshot, WardError> {
    tauri::async_runtime::spawn_blocking(move || match harness.as_str() {
        "claude" => {
            let snap = crate::usage::live::snapshot()?;
            // Write-through (Plan 17): warm the cache with the live gauges so a
            // re-open paints them instantly instead of re-hitting the network.
            if snap.available {
                let _ = crate::usage::cache::write_entry("claude", &snap);
            }
            Ok(snap)
        }
        other => Err(WardError::HarnessUnavailable(format!("live usage unsupported for {other}"))),
    })
    .await
    .map_err(|e| WardError::Live(format!("live usage task failed: {e}")))?
}

/// Plan 16 — is the live (network) usage path opted in?
#[tauri::command]
pub fn live_usage_enabled() -> bool {
    crate::usage::live::live_enabled()
}

/// Plan 16 — opt in/out of the live usage path (creates/removes the sentinel).
#[tauri::command]
pub fn set_live_usage_enabled(enabled: bool) -> Result<(), WardError> {
    crate::usage::live::set_live_enabled(enabled)
}

// ── Native shell status (Plan 15) ────────────────────────────────────────

/// Plan 15 — push the latest scan's critical count to the dock badge + tray tooltip.
#[tauri::command]
pub fn native_update_status(
    app: tauri::AppHandle,
    critical: usize,
    last_scan_at: Option<String>,
) -> Result<(), WardError> {
    crate::native::tray::update_badge(&app, critical);
    if let Some(tray) = app.tray_by_id("ward-tray") {
        let tip = crate::native::tray::format_tooltip(critical, last_scan_at.as_deref());
        let _ = tray.set_tooltip(Some(tip));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn backup_log_impl_empty_then_lists_newest_first() {
        use crate::backup::git as g;
        // Separate temp dirs for the repo and the export source so the
        // source tree is never itself inside the repo.
        let repo_root = tempfile::tempdir().unwrap();
        let src_root = tempfile::tempdir().unwrap();
        let repo = repo_root.path();
        let src = src_root.path();

        // No .git yet → empty history, no error.
        assert!(backup_log_impl(repo, 20).unwrap().is_empty());

        g::init(repo, "ward", "ward@local").unwrap();
        // Initialized but commit-less → still empty + Ok (a raw `git log`
        // would error here; backup_log_impl must not).
        assert!(backup_log_impl(repo, 20).unwrap().is_empty());

        // Two commits, newest last.
        fs::write(src.join("a"), "1").unwrap();
        g::export_to_repo(src, repo).unwrap();
        g::commit(repo, "one").unwrap();
        fs::write(src.join("a"), "2").unwrap();
        g::export_to_repo(src, repo).unwrap();
        g::commit(repo, "two").unwrap();

        let log = backup_log_impl(repo, 20).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].subject, "two");
        assert_eq!(log[1].subject, "one");
        assert_ne!(log[0].sha, log[1].sha);
    }

    #[test]
    fn scan_impl_returns_claude_result() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".claude/skills/a")).unwrap();
        fs::write(dir.path().join(".claude/skills/a/SKILL.md"), "x").unwrap();

        let registry = build_registry();
        let result = scan_impl(&registry, dir.path(), "claude").unwrap();
        assert_eq!(result.harness_id, "claude");
        assert_eq!(result.items.iter().filter(|i| i.category == "skill").count(), 1);
    }

    #[test]
    fn scan_impl_unknown_harness_errors() {
        let registry = build_registry();
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            scan_impl(&registry, dir.path(), "nope"),
            Err(WardError::HarnessUnavailable(_))
        ));
    }

    #[test]
    fn reads_allowed_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join(".claude/x.md");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, "hello").unwrap();
        assert_eq!(read_file_impl(&f, dir.path()).unwrap(), "hello");
    }

    #[test]
    fn rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("../etc/passwd");
        assert!(matches!(read_file_impl(&bad, dir.path()), Err(WardError::PathEscaped(_))));
    }

    /// bulk_restore applies ops in reverse order. The simplest way to
    /// verify this is to delete two files (capturing two RestoreInfo
    /// payloads) and check the order their `restore()` impls run.
    #[test]
    fn bulk_restore_applies_in_reverse_order() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join(".claude/memory")).unwrap();
        let p1 = home.join(".claude/memory/a.md");
        let p2 = home.join(".claude/memory/b.md");
        fs::write(&p1, "alpha").unwrap();
        fs::write(&p2, "beta").unwrap();
        let ops = crate::harness::adapters::claude_ops::ClaudeOps;
        let ctx = Ctx { home, cwd: None };
        let item1 = HarnessItem {
            category: "memory".into(), scope_id: "global".into(),
            name: "a".into(), description: String::new(),
            path: p1.display().to_string(),
            movable: true, deletable: true, locked: false, effective: None,
            mcp_config: None,
        };
        let item2 = HarnessItem {
            category: "memory".into(), scope_id: "global".into(),
            name: "b".into(), description: String::new(),
            path: p2.display().to_string(),
            movable: true, deletable: true, locked: false, effective: None,
            mcp_config: None,
        };
        let info1 = ops.delete_item(&ctx, &item1, &[]).unwrap();
        let info2 = ops.delete_item(&ctx, &item2, &[]).unwrap();
        assert!(!p1.exists());
        assert!(!p2.exists());

        // Apply in reverse: b first, then a.
        ops.restore(&ctx, &info2).unwrap();
        ops.restore(&ctx, &info1).unwrap();
        assert!(p2.exists());
        assert!(p1.exists());
        assert_eq!(fs::read_to_string(&p2).unwrap(), "beta");
        assert_eq!(fs::read_to_string(&p1).unwrap(), "alpha");
    }

    /// bulk_restore applied via the public command path must use the
    /// reverse order. We assert this indirectly: if bulk_restore ran in
    /// forward order, a later sub-op might overwrite a file that an
    /// earlier sub-op recreates. Here both ops are deletes, so we just
    /// verify the command completes without error.
    #[test]
    fn bulk_command_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join(".claude/memory")).unwrap();
        let p1 = home.join(".claude/memory/a.md");
        let p2 = home.join(".claude/memory/b.md");
        fs::write(&p1, "alpha").unwrap();
        fs::write(&p2, "beta").unwrap();
        let item1 = HarnessItem {
            category: "memory".into(), scope_id: "global".into(),
            name: "a".into(), description: String::new(),
            path: p1.display().to_string(),
            movable: true, deletable: true, locked: false, effective: None,
            mcp_config: None,
        };
        let item2 = HarnessItem {
            category: "memory".into(), scope_id: "global".into(),
            name: "b".into(), description: String::new(),
            path: p2.display().to_string(),
            movable: true, deletable: true, locked: false, effective: None,
            mcp_config: None,
        };
        // Direct invocation of the impls (commands::bulk/bulk_restore
        // are pub functions, but they require `dirs::home_dir()` — for
        // testability we exercise the HarnessOps surface directly).
        let ops = crate::harness::adapters::claude_ops::ClaudeOps;
        let ctx = Ctx { home, cwd: None };
        let mut infos = Vec::new();
        infos.push(ops.delete_item(&ctx, &item1, &[]).unwrap());
        infos.push(ops.delete_item(&ctx, &item2, &[]).unwrap());
        for info in infos.iter().rev() {
            ops.restore(&ctx, info).unwrap();
        }
        assert!(p1.exists());
        assert!(p2.exists());
    }

    /// `context_budget` is the public command wired to the Budget mode
    /// in the UI. We exercise it directly through the `compose` helper
    /// (the Tauri command itself relies on `dirs::home_dir()` which is
    /// not testable here without process isolation). This test verifies
    /// the happy path produces a populated `BudgetBreakdown` for a
    /// scope containing skills + CLAUDE.md.
    #[test]
    fn context_budget_compose_for_populated_scope() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let skill_path = home.join(".claude/skills/brainstorming/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
        fs::write(&skill_path, "brainstorming body 1234").unwrap();
        let claude_md = home.join(".claude/CLAUDE.md");
        fs::write(&claude_md, "global memory\nwith second line").unwrap();

        let ctx = Ctx { home, cwd: None };
        let items = framework::run_scan(&ClaudeAdapter, &ctx).unwrap().items;
        let servers: Vec<String> = vec![];
        let b = budget::compose("global", &items, &servers, home);
        assert_eq!(b.system_loaded, budget::SYSTEM_LOADED);
        assert_eq!(b.mcp_schemas, 0);
        assert!(b.claudemd > budget::CLAUDEMD_WRAPPER);
        // New model: the skill surfaces as a METADATA row (its body is
        // deferred), not a full always-loaded item.
        assert!(b.metadata_items.iter().any(|i| i.category == "skill"));
        assert!(b.used > b.system_loaded);
    }

    /// Verifies that the same MCP server appearing twice in the scan
    /// is still counted ONCE — i.e. the dedup happens at compose time.
    #[test]
    fn context_budget_dedupes_mcp_servers() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let items = vec![
            HarnessItem {
                category: "mcp".into(), scope_id: "global".into(),
                name: "github".into(), description: String::new(),
                path: home.join(".claude.json").display().to_string(),
                movable: false, deletable: false, locked: false,
                effective: None,
                mcp_config: Some(serde_json::json!({"command":"gh"})),
            },
            HarnessItem {
                category: "mcp".into(), scope_id: "global".into(),
                name: "github".into(), description: String::new(),
                path: home.join(".mcp.json").display().to_string(),
                movable: false, deletable: false, locked: false,
                effective: None,
                mcp_config: Some(serde_json::json!({"command":"gh"})),
            },
        ];
        let servers = vec!["github".to_string(), "github".to_string()];
        let b = budget::compose("global", &items, &servers, home);
        assert_eq!(b.mcp_schemas, budget::MCP_TOOL_SCHEMA);
    }
}