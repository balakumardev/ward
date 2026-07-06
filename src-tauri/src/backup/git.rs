//! Plan 08 — Backup git operations.
//!
//! Mirrors the layout of `~/.claude/` into a local git repo (the
//! "backup repo"), then exposes init/commit/sync/push/status/log
//! primitives that the Tauri commands and the launchd-triggered CLI
//! both call.
//!
//! Behaviour follows CCO's `src/backup-git.mjs`:
//!   - git is invoked via the system binary (shells out — no `git2`).
//!   - The repo lives under `~/.ward-backups/` (NOT `~/.claude-backups/`).
//!   - `push` is gated: with no remote configured it returns
//!     `PushResult { pushed: false, reason: "..." }`; the *user* must
//!     explicitly call `backup_push` to trigger a network action.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::WardError;

// ── Constants ──────────────────────────────────────────────────────────

/// Backup root inside the user's home dir. Different from CCO's
/// `~/.claude-backups/` — we ship Ward backup, not Claude backup.
pub const BACKUP_HOME_DIR: &str = ".ward-backups";

/// Identity fallback used when the host has no global git identity
/// configured. Set as **local** (repo-scoped) so we never pollute the
/// user's global git config.
pub const FALLBACK_USER_NAME: &str = "ward";
pub const FALLBACK_USER_EMAIL: &str = "ward@local";

/// Top-level files / dirs that intentionally live inside the backup
/// repo but should NOT be committed.
const REPO_GITIGNORE: &str = "backup.log\n";

// ── BackupDir ──────────────────────────────────────────────────────────

/// Resolve the user's Ward backup dir (`~/.ward-backups/`) and
/// ensure it exists. Returns the absolute path. Caller may store
/// this for the lifetime of the backup session — it never changes
/// within a single run.
pub struct BackupDir {
    pub path: PathBuf,
}

/// Where the backup repo lives for this caller's world view. When
/// called as a Tauri command we want the real `~/.ward-backbacks/`
/// (the user cares); tests pass an explicit `&Path` via the
/// `*_with_dir` family of helpers.
pub fn backup_dir() -> Result<BackupDir, WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    let path = home.join(BACKUP_HOME_DIR);
    ensure_dir(&path)?;
    Ok(BackupDir { path })
}

/// Like `backup_dir` but rooted at `root`. Test-only — production code
/// should call `backup_dir()`.
#[cfg(test)]
pub fn backup_dir_at(root: &Path) -> Result<BackupDir, WardError> {
    let path = root.join(BACKUP_HOME_DIR);
    ensure_dir(&path)?;
    Ok(BackupDir { path })
}

// ── Repo / Source paths ────────────────────────────────────────────────

/// Returns the absolute path of the backup repo (the dir containing
/// `.git`). We use the BackupDir's path directly as the repo root
/// because the `~/.ward-backups/` directory IS the git repo.
pub fn repo_dir(bd: &BackupDir) -> &Path {
    bd.path.as_path()
}

// ── Init ───────────────────────────────────────────────────────────────

/// Result of `git init` — `created` tells the caller whether the
/// `.git` folder was new (true) or pre-existing (false). The caller
/// may use this to skip the identity fallback wireup when the repo
/// already had a user set up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitResult {
    pub created: bool,
}

/// `git init` the backup repo at `repo_dir`, then ensure
/// `user.name` / `user.email` are set locally (so commits are
/// attributable even if the host has no global identity).
/// Writes a `.gitignore` that ignores `backup.log` so the launchd
/// scheduler's stdout/stderr capture never touches git history.
///
/// If the repo already exists we DON'T call `init` again — we only
/// guarantee that the identity is set and the `.gitignore` is in
/// place. That keeps this safe to call on every launch.
pub fn init(
    repo_dir: &Path,
    identity_name: &str,
    identity_email: &str,
) -> Result<InitResult, WardError> {
    let created = if !repo_dir.join(".git").exists() {
        git(repo_dir, &["init", "-b", "main"])?;
        true
    } else {
        false
    };

    // Always (re-)apply identity locally. `--local` keeps this scoped
    // to the repo so we never touch the host's global git config.
    set_local_identity(repo_dir, "user.name", identity_name)?;
    set_local_identity(repo_dir, "user.email", identity_email)?;

    ensure_gitignore(repo_dir)?;

    Ok(InitResult { created })
}

/// Set `user.name`/`user.email` using `ward <ward@local>` when no
/// identity is globally configured. The local-scope write inside
/// `init` always uses the *passed* identity — this helper is only
/// used by the public `backup_run` flow to pick a sensible default
/// for the "no global identity" case.
pub fn ensure_identity_or_fallback(
    repo_dir: &Path,
    fallback_name: &str,
    fallback_email: &str,
) -> Result<(), WardError> {
    let has_name = !git_capture_stdout(repo_dir, &["config", "--global", "--get", "user.name"])
        .map(|s| s.is_empty())
        .unwrap_or(true); // absent → use fallback
    let has_email = !git_capture_stdout(repo_dir, &["config", "--global", "--get", "user.email"])
        .map(|s| s.is_empty())
        .unwrap_or(true);

    if !has_name || !has_email {
        // At least one missing — set BOTH locally to the fallback.
        set_local_identity(repo_dir, "user.name", fallback_name)?;
        set_local_identity(repo_dir, "user.email", fallback_email)?;
    }
    Ok(())
}

// ── Export ─────────────────────────────────────────────────────────────

/// What `export_to_repo` produced. `files_copied` excludes dirs,
/// `skipped` collects entries the caller was promised to skip when
/// they didn't exist on disk (so the report is observable).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub files_copied: usize,
    pub bytes_copied: u64,
    pub skipped: Vec<String>,
}

/// Recursively copy the contents of `source` (e.g. `~/.claude/`)
/// into `repo_dir`, mirroring the relative layout. Missing entries
/// are recorded in `skipped`, never errored — the export is
/// best-effort. Returns the aggregate stats.
///
/// Symlinks are NOT followed — we read the link itself and skip
/// rather than recursing, to avoid copying outside the backup dir.
/// Nested `.git` directories are skipped entirely so embedded repos
/// under `~/.claude/` are never recorded as gitlinks (mode 160000).
pub fn export_to_repo(source: &Path, repo_dir: &Path) -> Result<ExportReport, WardError> {
    let mut report = ExportReport::default();
    if !source.exists() {
        return Ok(report);
    }
    ensure_dir(repo_dir)?;
    copy_tree(source, repo_dir, &mut report, true)?;
    Ok(report)
}

// ── Export filtering ────────────────────────────────────────────────────
//
// A config backup should capture what the user authored/configured (skills,
// memory, commands, agents, hooks, settings, CLAUDE.md, MCP config) — NOT the
// hundreds of MB of caches, cloned plugin repos, session transcripts, history,
// and machine-local state that `~/.claude/` also accumulates. Those are large,
// regenerable, and churn constantly, which made every backup slow (a ~770MB
// copy + `git add -A`) and noisy. The denylists below are matched on entry
// name during the copy walk.

/// Directory names skipped anywhere in the tree.
const EXCLUDED_DIRS: &[&str] = &[
    "cache", "debug", "daemon", "file-history", "projects", "security",
    "worktree-manager", "transcripts", "shell-snapshots", "usage-data",
    "statsig", "todos", "backups", "skills-backup", "ide", "logs", ".trash",
];

/// Under `plugins/`, these are cloned repos / caches — skip them but keep the
/// small top-level plugin config files. (`cache` is already in EXCLUDED_DIRS.)
const PLUGINS_EXCLUDED_SUBDIRS: &[&str] = &["repos", "cache", "marketplaces"];

/// Exact file names skipped anywhere — logs and machine-local state.
const EXCLUDED_FILES: &[&str] = &[
    "history.jsonl", ".DS_Store", ".last-cleanup", ".last-update-result.json",
    "daemon-auth-cooldown", "daemon-auth-status.json", "daemon.lock",
    "daemon.status.json",
];

/// Whether an entry named `name` (a directory when `is_dir`) sitting inside a
/// directory named `parent` should be excluded from the backup export.
fn is_excluded(name: &str, is_dir: bool, parent: Option<&str>) -> bool {
    if is_dir {
        EXCLUDED_DIRS.contains(&name)
            || (parent == Some("plugins") && PLUGINS_EXCLUDED_SUBDIRS.contains(&name))
    } else {
        EXCLUDED_FILES.contains(&name) || name.ends_with(".lock")
    }
}

/// `top` is true only for the immediate children of the export root, so the
/// UI-facing `skipped` list surfaces top-level exclusions (e.g. "security/
/// (excluded)") without the noise of every nested skip.
fn copy_tree(src: &Path, dst: &Path, report: &mut ExportReport, top: bool) -> Result<(), WardError> {
    if !src.exists() {
        report.skipped.push(src.display().to_string());
        return Ok(());
    }
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        report.skipped.push(src.display().to_string());
        return Ok(());
    }
    if meta.is_dir() {
        ensure_dir(dst)?;
        let parent = src.file_name().and_then(|n| n.to_str());
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            // Skip nested `.git` directories. Several dirs under
            // `~/.claude/` (e.g. `memory/`) are their own git repos;
            // copying their `.git/` would make the outer `git add -A`
            // record them as gitlinks (mode 160000) instead of backing
            // up the real files — silently losing the actual content.
            if entry.file_name() == ".git" {
                continue;
            }
            let raw_name = entry.file_name();
            let name = raw_name.to_string_lossy();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            // Drop caches / transcripts / cloned repos / machine-local state.
            if is_excluded(name.as_ref(), is_dir, parent) {
                if top {
                    report
                        .skipped
                        .push(format!("{}{} (excluded)", name, if is_dir { "/" } else { "" }));
                }
                continue;
            }
            let child_src = entry.path();
            let child_dst = dst.join(entry.file_name());
            copy_tree(&child_src, &child_dst, report, false)?;
        }
        return Ok(());
    }
    if meta.is_file() {
        if let Some(parent) = dst.parent() {
            ensure_dir(parent)?;
        }
        std::fs::copy(src, dst)?;
        report.files_copied += 1;
        report.bytes_copied += meta.len();
        return Ok(());
    }
    Ok(())
}

// ── Commit / Sync ──────────────────────────────────────────────────────

/// Wire form of a commit attempt. `committed = false` means there
/// was nothing to back up; `sha` is the new commit hash when
/// `committed = true`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub committed: bool,
    pub sha: Option<String>,
    pub message: String,
    pub committed_at: Option<DateTime<Utc>>,
}

/// `git add -A && git commit -m <message>`. Returns
/// `CommitInfo { committed: false, ... }` when the working tree has
/// no changes vs HEAD — i.e. nothing to back up.
pub fn commit(repo_dir: &Path, message: &str) -> Result<CommitInfo, WardError> {
    git(repo_dir, &["add", "-A"])?;
    git(repo_dir, &["diff", "--cached", "--quiet"])
        .map_err(|_| ()) // E -> "changes present"
        .ok();

    // Check staged diff explicitly:
    let has_staged = git_capture_stdout(repo_dir, &["diff", "--cached", "--name-only"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    if !has_staged {
        return Ok(CommitInfo {
            committed: false,
            sha: None,
            message: "no changes".to_string(),
            committed_at: None,
        });
    }

    git(repo_dir, &["commit", "-m", message])?;
    let sha = git_capture_stdout(repo_dir, &["rev-parse", "HEAD"])?
        .trim()
        .to_string();
    let committed_at = chrono::Utc::now();

    Ok(CommitInfo {
        committed: true,
        sha: Some(sha),
        message: message.to_string(),
        committed_at: Some(committed_at),
    })
}

// ── Push ───────────────────────────────────────────────────────────────

/// `git push` outcome. The UI uses `pushed = false` + `reason` to
/// explain why a push call was a no-op (no remote, offline, etc.).
/// `pushed = true` means a network action actually ran.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    pub pushed: bool,
    pub reason: String,
    pub remote_url: Option<String>,
}

/// Run `git push` against the repo's configured remote (typically
/// `origin`). If no remote is configured this is a no-op that
/// returns `pushed = false`. The caller — NOT this function — is
/// responsible for NOT calling this without an explicit user
/// action (per Plan 08 hard rule).
pub fn push(repo_dir: &Path) -> Result<PushResult, WardError> {
    let remote_url = remote_url(repo_dir);
    let remote = match remote_url.as_deref() {
        Some(_) => true,
        None => false,
    };
    if !remote {
        return Ok(PushResult {
            pushed: false,
            reason: "no remote configured".to_string(),
            remote_url: None,
        });
    }
    match git(repo_dir, &["push"]) {
        Ok(_) => Ok(PushResult {
            pushed: true,
            reason: "pushed".to_string(),
            remote_url,
        }),
        Err(e) => Ok(PushResult {
            pushed: false,
            reason: format!("push failed: {e}"),
            remote_url,
        }),
    }
}

pub fn remote_url(repo_dir: &Path) -> Option<String> {
    git_capture_stdout(repo_dir, &["remote", "get-url", "origin"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Add or update the `origin` remote to `url`. Idempotent.
pub fn set_remote(repo_dir: &Path, url: &str) -> Result<(), WardError> {
    if remote_url(repo_dir).is_some() {
        git(repo_dir, &["remote", "set-url", "origin", url])?;
    } else {
        git(repo_dir, &["remote", "add", "origin", url])?;
    }
    Ok(())
}

// ── Status / Log ───────────────────────────────────────────────────────

/// Snapshot of working-tree state vs HEAD.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub modified: usize,
    pub untracked: usize,
    pub clean: bool,
}

/// `git status --porcelain` rolled up into counts. The full porcelain
/// lines are NOT returned — only the summary, since the UI shows
/// them in a single badge.
pub fn status(repo_dir: &Path) -> Result<GitStatus, WardError> {
    let out = git_capture_stdout(repo_dir, &["status", "--porcelain"])?;
    let mut modified = 0usize;
    let mut untracked = 0usize;
    for line in out.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        // First two chars are the status codes; anything starting
        // with "??" is an untracked entry. Anything else is tracked
        // (M/A/D/R/...) = modified.
        let bytes = line.as_bytes();
        let prefix: String = bytes.iter().take(2).map(|b| *b as char).collect();
        if prefix == "??" {
            untracked += 1;
        } else {
            modified += 1;
        }
    }
    Ok(GitStatus {
        modified,
        untracked,
        clean: modified + untracked == 0,
    })
}

/// A single commit from `git log`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitLogEntry {
    pub sha: String,
    pub subject: String,
    pub author: String,
    pub committed_at: DateTime<Utc>,
}

/// Last `n` commit entries, newest first.
pub fn log(repo_dir: &Path, n: usize) -> Result<Vec<GitLogEntry>, WardError> {
    let fmt = "%H%x1f%an%x1f%ai%x1f%s";
    let n_arg = format!("-{}", n.max(1));
    let out = git_capture_stdout(repo_dir, &["log", &n_arg, &format!("--format={fmt}")])?;
    let mut entries = Vec::new();
    for line in out.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\u{1f}').collect();
        if parts.len() != 4 {
            continue;
        }
        let committed_at = DateTime::parse_from_rfc3339(parts[2])
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        entries.push(GitLogEntry {
            sha: parts[0].to_string(),
            author: parts[1].to_string(),
            committed_at,
            subject: parts[3].to_string(),
        });
    }
    Ok(entries)
}

/// Return the SHA + UTC timestamp of the most recent commit, if any.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LastCommit {
    pub sha: String,
    pub committed_at: DateTime<Utc>,
}

pub fn last_commit(repo_dir: &Path) -> Result<Option<LastCommit>, WardError> {
    let fmt = "%H%x1f%ai";
    let out = match git_capture_stdout(repo_dir, &["log", "-1", &format!("--format={fmt}")]) {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };
    let line = out.lines().next().unwrap_or("");
    let parts: Vec<&str> = line.split('\u{1f}').collect();
    if parts.len() != 2 {
        return Ok(None);
    }
    let committed_at = DateTime::parse_from_rfc3339(parts[1])
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Ok(Some(LastCommit {
        sha: parts[0].to_string(),
        committed_at,
    }))
}

// ── Git binary plumbing ────────────────────────────────────────────────

fn ensure_dir(p: &Path) -> Result<(), WardError> {
    if !p.exists() {
        std::fs::create_dir_all(p)?;
    }
    Ok(())
}

fn set_local_identity(repo_dir: &Path, key: &str, value: &str) -> Result<(), WardError> {
    git(repo_dir, &["config", "--local", key, value])
}

fn ensure_gitignore(repo_dir: &Path) -> Result<(), WardError> {
    let path = repo_dir.join(".gitignore");
    if !path.exists() {
        std::fs::write(path, REPO_GITIGNORE)?;
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Result<(), WardError> {
    let output = run_git(dir, args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(WardError::Git(format!(
            "git {args:?} (cwd={}): {detail}",
            dir.display()
        )));
    }
    Ok(())
}

pub(crate) fn git_capture_stdout(dir: &Path, args: &[&str]) -> Result<String, WardError> {
    let output = run_git(dir, args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(WardError::Git(format!(
            "git {args:?} (cwd={}): {detail}",
            dir.display()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_git(dir: &Path, args: &[&str]) -> Result<Output, WardError> {
    // Resolve `git` to an absolute path on first call. macOS sandboxed
    // shells sometimes refuse Command::new("git") even when `which git`
    // succeeds — passing the resolved binary avoids that class of bug
    // and also lets us report a useful error when git is genuinely missing.
    static GIT_BIN: std::sync::OnceLock<Result<PathBuf, WardError>> = std::sync::OnceLock::new();
    let bin = GIT_BIN
        .get_or_init(|| {
            which_git().ok_or_else(|| WardError::Git("git binary not found on PATH".into()))
        })
        .as_ref()
        .map_err(|e| WardError::Git(e.to_string()))?
        .clone();
    let output = Command::new(bin)
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    Ok(output)
}

fn which_git() -> Option<PathBuf> {
    // Find an absolute path to `git`. The cargo test binary, when
    // launched from inside hooks/sandboxes, sometimes refuses to
    // resolve `Command::new("git")` even though bare `Command::new(c)`
    // with `c` already absolute works fine. So we resolve the absolute
    // path with `which`-style fallback and ONLY return absolute paths.
    //
    // We prefer the user's PATH — `/usr/bin/git` is the macOS-bundled
    // binary but a Homebrew/xcode-select-managed `git` in `/opt/...`
    // or `/usr/local/bin` may legitimately be newer. We try a small
    // ordered list of well-known absolute locations so we still
    // succeed if PATH is sandboxed.
    let well_known = [
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
        "/usr/bin/git",
    ];
    for c in well_known.iter() {
        if std::path::Path::new(c).exists() {
            // Verify it actually runs.
            if std::process::Command::new(c).arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
                return Some(PathBuf::from(c));
            }
        }
    }
    // Fall back to whatever `which`-like resolution finds on PATH,
    // but only accept absolute results.
    if let Ok(out) = std::process::Command::new("/usr/bin/env").args(["which", "git"]).output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                let p = PathBuf::from(&s);
                if p.is_absolute() {
                    return Some(p);
                }
            }
        }
    }
    None
}

// ── Tests ──────────────────────────────────────────────────────────────
//
// All tests use a per-test tempdir for both source and backup-repo
// so they never touch the user's real `~/.ward-backups/`.

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: per-test temp dirs.
    /// Returns `(source, backup_root, bd)`. The caller MUST keep
    /// `backup_root` alive for the duration of the test — dropping
    /// it would delete `bd.path`.
    fn make_world() -> (tempfile::TempDir, tempfile::TempDir, BackupDir) {
        let source_root = tempfile::tempdir().unwrap();
        let backup_root = tempfile::tempdir().unwrap();
        let bd = backup_dir_at(backup_root.path()).unwrap();
        std::fs::create_dir_all(source_root.path().join(".claude/agents")).unwrap();
        (source_root, backup_root, bd)
    }

    #[test]
    fn which_git_resolves_to_absolute_path() {
        let bin = super::which_git();
        assert!(bin.is_some(), "git binary should resolve inside the cargo test env");
        let bin = bin.unwrap();
        // Must be an absolute path (we never want to rely on PATH lookup
        // from inside this binary — see `run_git`).
        assert!(bin.is_absolute());
        assert!(bin.exists());
    }

    #[test]
    fn run_git_with_absolute_path_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        // Use the resolved binary directly so we exercise the same
        // spawn path the production code uses.
        let bin = super::which_git().expect("git must be available");
        let out = std::process::Command::new(bin)
            .args(["init", "-b", "main"])
            .current_dir(tmp.path())
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("git init via absolute path should work");
        assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        assert!(tmp.path().join(".git").exists());
    }

    #[test]
    fn init_creates_repo_and_identity() {
        let (_src, _backup_root, bd) = make_world();
        let r = init(repo_dir(&bd), "ward", "ward@local").unwrap();
        assert!(r.created);
        assert!(repo_dir(&bd).join(".git").exists());
        assert!(repo_dir(&bd).join(".gitignore").exists());

        let name = git_capture_stdout(repo_dir(&bd), &["config", "--local", "--get", "user.name"]).unwrap();
        let email = git_capture_stdout(repo_dir(&bd), &["config", "--local", "--get", "user.email"]).unwrap();
        assert_eq!(name.trim(), "ward");
        assert_eq!(email.trim(), "ward@local");

        // .gitignore should ignore backup.log
        let gi = fs::read_to_string(repo_dir(&bd).join(".gitignore")).unwrap();
        assert!(gi.contains("backup.log"));
    }

    #[test]
    fn init_is_idempotent() {
        let (_src, _backup_root, bd) = make_world();
        init(repo_dir(&bd), "ward", "ward@local").unwrap();
        let r2 = init(repo_dir(&bd), "different", "diff@local").unwrap();
        assert!(!r2.created);
        let name = git_capture_stdout(repo_dir(&bd), &["config", "--local", "--get", "user.name"]).unwrap();
        assert_eq!(name.trim(), "different");
    }

    #[test]
    fn export_copies_files_recursively() {
        let src = tempfile::tempdir().unwrap();
        let backup_root = tempfile::tempdir().unwrap();
        let bd = backup_dir_at(backup_root.path()).unwrap();
        init(repo_dir(&bd), "ward", "ward@local").unwrap();

        fs::create_dir_all(src.path().join(".claude/agents")).unwrap();
        fs::write(src.path().join(".claude/CLAUDE.md"), "hello").unwrap();
        fs::write(src.path().join(".claude/agents/a.md"), "alpha").unwrap();

        let report = export_to_repo(&src.path().join(".claude"), repo_dir(&bd)).unwrap();
        assert_eq!(report.files_copied, 2);
        assert_eq!(report.skipped.len(), 0);
        assert!(report.bytes_copied > 0);
        assert!(repo_dir(&bd).join("CLAUDE.md").exists());
        assert!(repo_dir(&bd).join("agents/a.md").exists());
    }

    #[test]
    fn export_skips_nested_git_dirs() {
        // Regression: several dirs under `~/.claude/` (e.g. `memory/`) are
        // themselves git repos. If copy_tree recurses into their `.git/`,
        // the outer `git add -A` records each as a gitlink (mode 160000)
        // instead of backing up the real files — so those files are
        // silently NOT backed up (only an unrestorable SHA).
        //
        // NOTE: this test builds a REAL committed nested repo. A bare
        // `.git/HEAD` file is NOT sufficient to reproduce the bug — git
        // only records a gitlink when `.git` resolves to a valid
        // repository, so a stub `HEAD` would pass both before and after
        // the fix (verified empirically). The real repo makes the test
        // genuinely fail before the copy_tree `.git` skip and pass after.
        let src = tempfile::tempdir().unwrap();
        let backup_root = tempfile::tempdir().unwrap();
        let bd = backup_dir_at(backup_root.path()).unwrap();
        init(repo_dir(&bd), "ward", "ward@local").unwrap();

        // Build ~/.claude/memory/ as its own committed git repo.
        let mem = src.path().join(".claude/memory");
        fs::create_dir_all(&mem).unwrap();
        fs::write(mem.join("note.md"), "important memory\n").unwrap();
        git(&mem, &["init", "-b", "main"]).unwrap();
        git(&mem, &["config", "--local", "user.name", "nested"]).unwrap();
        git(&mem, &["config", "--local", "user.email", "nested@local"]).unwrap();
        git(&mem, &["add", "-A"]).unwrap();
        git(&mem, &["commit", "-m", "seed"]).unwrap();
        assert!(mem.join(".git").exists());

        // Export the ~/.claude tree into the backup repo, then commit.
        export_to_repo(&src.path().join(".claude"), repo_dir(&bd)).unwrap();
        let ci = commit(repo_dir(&bd), "backup").unwrap();
        assert!(ci.committed);

        // The nested repo's real file must be backed up …
        assert!(
            repo_dir(&bd).join("memory/note.md").exists(),
            "nested repo's real file must be copied into the backup"
        );
        // … its `.git` dir must NOT have been copied …
        assert!(
            !repo_dir(&bd).join("memory/.git").exists(),
            "nested .git dir must be skipped, never copied"
        );
        // … and NOTHING may be recorded as a gitlink (mode 160000).
        let ls = git_capture_stdout(repo_dir(&bd), &["ls-files", "-s"]).unwrap();
        let gitlinks = ls.lines().filter(|l| l.starts_with("160000")).count();
        assert_eq!(gitlinks, 0, "no gitlinks allowed; ls-files -s:\n{ls}");
        // Positive: the file is tracked as a normal blob.
        assert!(
            ls.lines().any(|l| l.contains("memory/note.md")),
            "memory/note.md must be tracked as a blob; ls-files -s:\n{ls}"
        );
    }

    #[test]
    fn export_skips_missing_source() {
        let _src = tempfile::tempdir().unwrap();
        let backup_root = tempfile::tempdir().unwrap();
        let bd = backup_dir_at(backup_root.path()).unwrap();
        init(repo_dir(&bd), "ward", "ward@local").unwrap();

        let report = export_to_repo(
            &std::path::PathBuf::from("/nope/does/not/exist"),
            repo_dir(&bd),
        )
        .unwrap();
        assert_eq!(report.files_copied, 0);
        assert_eq!(report.skipped.len(), 0);
    }

    #[test]
    fn commit_creates_commit_when_changes() {
        let src = tempfile::tempdir().unwrap();
        let backup_root = tempfile::tempdir().unwrap();
        let bd = backup_dir_at(backup_root.path()).unwrap();
        init(repo_dir(&bd), "ward", "ward@local").unwrap();

        fs::write(src.path().join("CLAUDE.md"), "hi").unwrap();
        export_to_repo(&src.path(), repo_dir(&bd)).unwrap();
        let r1 = commit(repo_dir(&bd), "first").unwrap();
        assert!(r1.committed);
        assert!(r1.sha.as_ref().unwrap().len() >= 7);
        assert!(r1.committed_at.is_some());

        // Re-running with no changes returns committed=false.
        let r2 = commit(repo_dir(&bd), "again").unwrap();
        assert!(!r2.committed);
        assert!(r2.sha.is_none());
    }

    #[test]
    fn push_returns_no_remote_when_unconfigured() {
        let (_src, _backup_root, bd) = make_world();
        init(repo_dir(&bd), "ward", "ward@local").unwrap();

        let r = push(repo_dir(&bd)).unwrap();
        assert!(!r.pushed);
        assert_eq!(r.reason, "no remote configured");
        assert!(r.remote_url.is_none());
    }

    #[test]
    fn set_remote_then_push_tries_network() {
        let (_src, _backup_root, bd) = make_world();
        init(repo_dir(&bd), "ward", "ward@local").unwrap();

        // Wire up a remote that obviously won't work (file:// path).
        let fake_remote = backup_root_for(&bd).join("fake.git");
        let url = format!("file://{}", fake_remote.display());
        set_remote(repo_dir(&bd), &url).unwrap();
        assert_eq!(remote_url(repo_dir(&bd)).as_deref(), Some(url.as_str()));

        // The push WILL fail because the fake remote doesn't exist —
        // the test asserts that the function attempts a real network
        // call rather than no-op'ing.
        let r = push(repo_dir(&bd)).unwrap();
        assert!(!r.pushed);
        assert!(r.reason.contains("push failed") || r.reason.contains("pushed"));
    }

    #[test]
    fn status_clean_then_modified() {
        let src = tempfile::tempdir().unwrap();
        let backup_root = tempfile::tempdir().unwrap();
        let bd = backup_dir_at(backup_root.path()).unwrap();
        init(repo_dir(&bd), "ward", "ward@local").unwrap();

        // No export yet → clean from the very first commit we make.
        fs::write(src.path().join("a"), "x").unwrap();
        export_to_repo(&src.path(), repo_dir(&bd)).unwrap();
        commit(repo_dir(&bd), "first").unwrap();
        let s = status(repo_dir(&bd)).unwrap();
        assert!(s.clean);

        // Mutate source and export again → modified.
        fs::write(src.path().join("a"), "y").unwrap();
        export_to_repo(&src.path(), repo_dir(&bd)).unwrap();
        let s = status(repo_dir(&bd)).unwrap();
        assert!(!s.clean);
        assert!(s.modified >= 1);
    }

    #[test]
    fn log_returns_commits_newest_first() {
        let src = tempfile::tempdir().unwrap();
        let backup_root = tempfile::tempdir().unwrap();
        let bd = backup_dir_at(backup_root.path()).unwrap();
        init(repo_dir(&bd), "ward", "ward@local").unwrap();

        fs::write(src.path().join("a"), "1").unwrap();
        export_to_repo(&src.path(), repo_dir(&bd)).unwrap();
        commit(repo_dir(&bd), "one").unwrap();
        fs::write(src.path().join("a"), "2").unwrap();
        export_to_repo(&src.path(), repo_dir(&bd)).unwrap();
        commit(repo_dir(&bd), "two").unwrap();

        let log = log(repo_dir(&bd), 10).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].subject, "two");
        assert_eq!(log[1].subject, "one");
        // No two distinct commits can share a SHA.
        assert_ne!(log[0].sha, log[1].sha);
    }

    #[test]
    fn backup_dir_path_is_under_user_home() {
        let (_src, _backup_root, bd) = make_world();
        // `.ward-backups/` path = BACKUP_HOME_DIR inside the root we gave it.
        assert!(repo_dir(&bd).ends_with(BACKUP_HOME_DIR));
        assert_eq!(repo_dir(&bd).file_name().and_then(|n| n.to_str()), Some(BACKUP_HOME_DIR));
    }

    /// Helper for tests that need the *parent* of the backup dir.
    fn backup_root_for(bd: &BackupDir) -> PathBuf {
        bd.path.parent().unwrap().to_path_buf()
    }
}
