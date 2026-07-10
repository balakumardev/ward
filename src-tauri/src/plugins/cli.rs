//! Plan 28 — `claude` binary resolver + `claude plugin …` CLI wrappers.
//!
//! Ward's Plugins mode drives Claude Code's plugin subsystem by shelling
//! out to the `claude plugin …` CLI (there is no on-disk write path for
//! plugin install/uninstall/marketplace mutations — the CLI owns that).
//!
//! The hard part is *finding* `claude`. A bundled Tauri app launched from
//! Finder inherits a minimal PATH that almost never contains the user's
//! npm-global / `~/.claude/local` install of `claude`. So this module
//! ports the robust GUI-app resolver from `backup/git.rs` (`which_git` /
//! `run_git`): try a fixed list of well-known absolute install locations
//! (each verified by actually running `--version`), then fall back to a
//! `which`-style lookup that only accepts an absolute result, cache the
//! answer, and always spawn via the *resolved absolute path* — macOS
//! sandboxed shells sometimes refuse `Command::new("claude")` even when
//! `which claude` succeeds.
//!
//! Split, per Ward's scheduler `write_plist`/`load_plist` convention:
//! the *pure arg builders* are unit-tested here; the *spawn wrappers*
//! that actually run `claude` are not (spawning the real CLI is out of
//! scope for units — it would require `claude` present and would mutate
//! the host's real plugin state).
//!
//! SECURITY: every wrapper spawns `Command::new(bin).args(&[…])` with
//! separate string arguments. No arg is ever interpolated into a shell
//! string and nothing goes through `sh -c`, so user-typed marketplace
//! sources / plugin names cannot inject shell metacharacters.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use crate::error::WardError;

// ── Resolver ───────────────────────────────────────────────────────────

/// Well-known absolute install locations for `claude`, in priority order.
/// `~` is expanded from `$HOME`. Homebrew (`/opt/homebrew`, `/usr/local`)
/// and the npm-global / `~/.local` bins cover the common install paths a
/// Finder-launched app's minimal PATH would miss.
fn candidate_paths() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(h) = home.as_ref() {
        out.push(h.join(".claude/local/claude"));
    }
    out.push(PathBuf::from("/opt/homebrew/bin/claude"));
    out.push(PathBuf::from("/usr/local/bin/claude"));
    if let Some(h) = home.as_ref() {
        out.push(h.join(".local/bin/claude"));
        out.push(h.join(".npm-global/bin/claude"));
    }
    out
}

/// Does `bin --version` run and exit successfully? Used to verify a
/// candidate is a real, runnable `claude` and not a dangling path.
fn runs_ok(bin: &std::path::Path) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Resolve `claude` to an absolute path. Tries the well-known locations
/// first (each verified by `--version`), then falls back to
/// `/usr/bin/env which claude`, accepting ONLY an absolute result. Never
/// returns a bare / relative path — see the module docs for why. `None`
/// means `claude` is genuinely not installed / not runnable.
fn resolve_claude() -> Option<PathBuf> {
    for c in candidate_paths() {
        if c.exists() && runs_ok(&c) {
            return Some(c);
        }
    }
    // Fall back to a `which`-style lookup on the inherited PATH, but only
    // trust an absolute result (a relative one would re-introduce the
    // sandbox-refusal bug the absolute-path spawn exists to avoid).
    if let Ok(out) = Command::new("/usr/bin/env").args(["which", "claude"]).output() {
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

/// Absolute path to the `claude` binary, resolved once and cached for the
/// process lifetime. `None` when `claude` isn't installed.
pub fn which_claude() -> Option<PathBuf> {
    static CLAUDE_BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    CLAUDE_BIN.get_or_init(resolve_claude).clone()
}

/// True when a runnable `claude` binary was resolved. Gates the Plugins
/// mode's install/uninstall/marketplace actions in the frontend.
pub fn claude_available() -> bool {
    which_claude().is_some()
}

// ── Pure arg builders (unit-tested, no spawn) ──────────────────────────

/// `claude plugin install <plugin>@<marketplace> --scope <scope>`.
pub fn install_args(plugin: &str, marketplace: &str, scope: &str) -> Vec<String> {
    vec![
        "plugin".to_string(),
        "install".to_string(),
        format!("{plugin}@{marketplace}"),
        "--scope".to_string(),
        scope.to_string(),
    ]
}

/// `claude plugin uninstall <plugin> --scope <scope> -y`. The trailing
/// `-y` is REQUIRED — without it the CLI prompts interactively and the
/// spawned process would hang forever with no TTY.
pub fn uninstall_args(plugin: &str, scope: &str) -> Vec<String> {
    vec![
        "plugin".to_string(),
        "uninstall".to_string(),
        plugin.to_string(),
        "--scope".to_string(),
        scope.to_string(),
        "-y".to_string(),
    ]
}

/// `claude plugin marketplace add <src> --scope <scope>`. `src` is a
/// user-typed marketplace source (a GitHub `owner/repo`, a URL, or a
/// local path) — passed as a single arg, never through a shell.
pub fn marketplace_add_args(src: &str, scope: &str) -> Vec<String> {
    vec![
        "plugin".to_string(),
        "marketplace".to_string(),
        "add".to_string(),
        src.to_string(),
        "--scope".to_string(),
        scope.to_string(),
    ]
}

/// `claude plugin marketplace update [<name>]`. With `None`, updates every
/// known marketplace; with `Some(name)`, just that one.
pub fn marketplace_update_args(name: Option<&str>) -> Vec<String> {
    let mut v = vec![
        "plugin".to_string(),
        "marketplace".to_string(),
        "update".to_string(),
    ];
    if let Some(n) = name {
        v.push(n.to_string());
    }
    v
}

/// `claude plugin list --json`.
pub fn list_json_args() -> Vec<String> {
    vec!["plugin".to_string(), "list".to_string(), "--json".to_string()]
}

/// `claude plugin marketplace list --json`.
pub fn marketplace_list_json_args() -> Vec<String> {
    vec![
        "plugin".to_string(),
        "marketplace".to_string(),
        "list".to_string(),
        "--json".to_string(),
    ]
}

// ── Spawn wrappers ─────────────────────────────────────────────────────

/// Run `claude <args…>` via the resolved absolute binary and return its
/// stdout on success. `args` are passed as separate strings (no shell),
/// so user input in `args` cannot inject shell metacharacters.
///
/// Errors:
///   - `claude` not resolvable → `WardError::Plugin("claude CLI not found on PATH")`.
///   - spawn failure (e.g. permission) → `WardError::Plugin(<io error>)`.
///   - non-zero exit → `WardError::Plugin(<stderr>)` (falls back to
///     stdout when stderr is empty so the error is never a blank string).
pub fn run_claude(args: &[String]) -> Result<String, WardError> {
    let bin =
        which_claude().ok_or_else(|| WardError::Plugin("claude CLI not found on PATH".into()))?;
    let output = Command::new(&bin)
        .args(args)
        // Pass the current process PATH through so `claude` can find any
        // helpers it shells out to (git/node). We already spawn via the
        // absolute `bin`, so `claude` itself is found regardless of PATH.
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .output()
        .map_err(|e| WardError::Plugin(format!("failed to run claude: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(WardError::Plugin(detail));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Install `<plugin>@<marketplace>` at `scope`.
pub fn install(plugin: &str, marketplace: &str, scope: &str) -> Result<String, WardError> {
    run_claude(&install_args(plugin, marketplace, scope))
}

/// Uninstall `<plugin>` at `scope` (non-interactive).
pub fn uninstall(plugin: &str, scope: &str) -> Result<String, WardError> {
    run_claude(&uninstall_args(plugin, scope))
}

/// Add marketplace `src` at `scope`.
pub fn marketplace_add(src: &str, scope: &str) -> Result<String, WardError> {
    run_claude(&marketplace_add_args(src, scope))
}

/// Update marketplace `name` (or all when `None`).
pub fn marketplace_update(name: Option<&str>) -> Result<String, WardError> {
    run_claude(&marketplace_update_args(name))
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_args_pins_scope_and_key() {
        let args = install_args("code-formatter", "claude-plugins-official", "user");
        assert_eq!(
            args,
            vec![
                "plugin",
                "install",
                "code-formatter@claude-plugins-official",
                "--scope",
                "user",
            ]
        );
    }

    #[test]
    fn uninstall_args_is_noninteractive() {
        let args = uninstall_args("code-formatter", "user");
        assert_eq!(
            args,
            vec!["plugin", "uninstall", "code-formatter", "--scope", "user", "-y"]
        );
        // The `-y` flag is what makes the uninstall non-interactive — a
        // missing `-y` would hang the spawned CLI waiting on a prompt.
        assert!(args.iter().any(|a| a == "-y"), "uninstall must be non-interactive");
    }

    #[test]
    fn marketplace_add_args_shape() {
        let args = marketplace_add_args("anthropics/claude-plugins", "user");
        assert_eq!(
            args,
            vec![
                "plugin",
                "marketplace",
                "add",
                "anthropics/claude-plugins",
                "--scope",
                "user",
            ]
        );
    }

    #[test]
    fn marketplace_update_args_with_and_without_name() {
        assert_eq!(
            marketplace_update_args(None),
            vec!["plugin", "marketplace", "update"]
        );
        assert_eq!(
            marketplace_update_args(Some("claude-plugins-official")),
            vec!["plugin", "marketplace", "update", "claude-plugins-official"]
        );
    }

    #[test]
    fn list_json_args_shape() {
        assert_eq!(list_json_args(), vec!["plugin", "list", "--json"]);
        assert_eq!(
            marketplace_list_json_args(),
            vec!["plugin", "marketplace", "list", "--json"]
        );
    }

    #[test]
    fn which_claude_returns_absolute_or_none() {
        // Tolerant of CI / machines without `claude` installed: a `None`
        // result is fine, but any `Some` MUST be an absolute path (we never
        // rely on PATH lookup from inside the spawned binary — see the
        // `which_git` port in backup/git.rs for why).
        if let Some(p) = which_claude() {
            assert!(p.is_absolute(), "resolved claude path must be absolute: {p:?}");
        }
    }
}
