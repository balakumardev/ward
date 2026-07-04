//! Plan 08 — Backup Center scheduler.
//!
//! macOS launchd only. Installs a per-user LaunchAgent that runs the
//! bundled Ward CLI with `--backup-once` on a fixed interval. This is
//! the ONLY persistent network/server-style integration Ward ships
//! — the `backup_push` command it triggers at run-time is still a
//! gated action the user opts into on demand.
//!
//! Scheme (mirrors CCO's `backup-scheduler.mjs` launchd branch with
//! `dev.balakumar.ward` branding):
//!   - Label:  `dev.balakumar.ward.backup`
//!   - Plist:  `~/Library/LaunchAgents/dev.balakumar.ward.backup.plist`
//!   - Invokes: <Ward.app/Contents/MacOS/ward --backup-once <scan_target>>
//!
//! The systemd branch from CCO is intentionally dropped. Ward is
//! macOS-only.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::WardError;
use crate::backup::git::BACKUP_HOME_DIR;

// ── Constants ──────────────────────────────────────────────────────────

/// Bundle id + label prefix used everywhere in this module. MUST stay
/// `dev.balakumar.ward` — the launchd label is recorded into the
/// LaunchAgent registry and renaming it would orphan existing
/// installs.
pub const BUNDLE_PREFIX: &str = "dev.balakumar.ward";

/// LaunchAgent label for the backup job.
pub const SCHEDULER_LABEL: &str = "dev.balakumar.ward.backup";

/// LaunchAgents directory under the user's home. macOS convention.
const LAUNCH_AGENTS_SUBDIR: &str = "Library/LaunchAgents";

/// Plist filename — `<label>.plist`.
pub const PLIST_FILENAME: &str = "dev.balakumar.ward.backup.plist";

/// Hard rules:
///   - Lower bound: 300s (5min). Anything faster would cause `launchd`
///     to throttle, plus no human-facing backup needs that cadence.
///   - Upper bound: 86_400s (24h). Beyond a day `launchd` is the
///     wrong tool — the user wants a real cron.
pub const MIN_INTERVAL_SECONDS: u32 = 300;
pub const MAX_INTERVAL_SECONDS: u32 = 86_400;

// ── SchedulerStatus ────────────────────────────────────────────────────

/// What `status()` reports. `Installed` means the plist file exists
/// AND `launchctl list` finds it; `NotInstalled` means neither; the
/// error variant surfaces split-brain (e.g. plist deleted but launchd
/// still has it loaded after a manual edit).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SchedulerStatus {
    Installed { interval_seconds: u32 },
    NotInstalled,
    Error(String),
}

impl SchedulerStatus {
    pub fn installed(&self) -> bool {
        matches!(self, SchedulerStatus::Installed { .. })
    }
}

// ── Paths ──────────────────────────────────────────────────────────────

/// Absolute path to the LaunchAgents plist for the current user.
/// Tests should use [`plist_path_in`] to point at a temp dir.
pub fn plist_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    home.join(LAUNCH_AGENTS_SUBDIR).join(PLIST_FILENAME)
}

/// Plist path under an arbitrary root. Test-only entry point — the
/// production `install` writes to the real LaunchAgents dir.
#[cfg(test)]
pub fn plist_path_in(root: &Path) -> PathBuf {
    root.join(LAUNCH_AGENTS_SUBDIR).join(PLIST_FILENAME)
}

// ── Interval validation ────────────────────────────────────────────────

/// Reject intervals outside the supported band so we don't get
/// `launchd` throttling (too low) or useless (too high).
pub fn validate_interval(n: u32) -> Result<(), WardError> {
    if n < MIN_INTERVAL_SECONDS {
        return Err(WardError::Backup(format!(
            "interval {n}s below minimum of {MIN_INTERVAL_SECONDS}s (5 minutes)"
        )));
    }
    if n > MAX_INTERVAL_SECONDS {
        return Err(WardError::Backup(format!(
            "interval {n}s above maximum of {MAX_INTERVAL_SECONDS}s (24 hours)"
        )));
    }
    Ok(())
}

// ── Plist content ──────────────────────────────────────────────────────

/// Build the LaunchAgent plist XML. Pure function — easy to unit test.
///
/// `ward_binary_path` is the bundled Ward CLI; typically
/// `/Applications/Ward.app/Contents/MacOS/ward`. `scan_target` is
/// passed verbatim as the only CLI arg so the scheduled run knows
/// which harness to back up. `~/.ward-backups/backup.log` is appended
/// with both stdout and stderr.
pub fn plist_content(
    interval_seconds: u32,
    ward_binary_path: &Path,
    scan_target: &Path,
) -> String {
    validate_interval(interval_seconds).expect("plist_content requires a validated interval");

    let label = SCHEDULER_LABEL;
    let prog = shell_escape(ward_binary_path.to_string_lossy().as_ref());
    let arg = shell_escape(scan_target.to_string_lossy().as_ref());
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let log_path = home.join(BACKUP_HOME_DIR).join("backup.log");
    let log_path_str = shell_escape(log_path.to_string_lossy().as_ref());

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{prog}</string>
    <string>--backup-once</string>
    <string>{arg}</string>
  </array>
  <key>StartInterval</key>
  <integer>{interval_seconds}</integer>
  <key>RunAtLoad</key>
  <false/>
  <key>StandardOutPath</key>
  <string>{log_path_str}</string>
  <key>StandardErrorPath</key>
  <string>{log_path_str}</string>
</dict>
</plist>
"#,
    )
}

/// Minimal positional escape for paths inside `<string>` entries —
/// launchd treats the value as the literal argv element, so a path
/// with spaces must be either quoted (preferred) or have its chars
/// escaped. We don't actually need escaping if we quote, but quoting
/// inside XML `<string>` is non-trivial — instead we keep the path as
/// a single token and reject control characters outright.
fn shell_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' | '>' | '&' => {
                // XML-significant — replace with the numeric refs so
                // the plist stays valid even when the path embeds them.
                let r = match c {
                    '<' => "&lt;",
                    '>' => "&gt;",
                    '&' => "&amp;",
                    _ => unreachable!(),
                };
                out.push_str(r);
            }
            c if (c as u32) < 0x20 => {
                // Drop control chars — launchd will reject them in
                // argv anyway, and they could break XML parsing.
            }
            c => out.push(c),
        }
    }
    out
}

// ── Install / Remove / Status ──────────────────────────────────────────

/// Write the plist under `~/Library/LaunchAgents/` and load it via
/// `launchctl`. Idempotent — re-running replaces an existing job
/// cleanly (unload, write, load).
pub fn install(
    interval_seconds: u32,
    ward_binary_path: &Path,
    scan_target: &Path,
) -> Result<(), WardError> {
    validate_interval(interval_seconds)?;
    let path = plist_path();
    write_and_load(&path, interval_seconds, ward_binary_path, scan_target)
}

/// Test-only install that points at a temp LaunchAgents dir. Returns
/// the plist path written so tests can inspect it.
#[cfg(test)]
pub fn install_at(
    root: &Path,
    interval_seconds: u32,
    ward_binary_path: &Path,
    scan_target: &Path,
) -> Result<PathBuf, WardError> {
    validate_interval(interval_seconds)?;
    let path = plist_path_in(root);
    write_and_load(&path, interval_seconds, ward_binary_path, scan_target)?;
    Ok(path)
}

fn write_and_load(
    path: &Path,
    interval_seconds: u32,
    ward_binary_path: &Path,
    scan_target: &Path,
) -> Result<(), WardError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let xml = plist_content(interval_seconds, ward_binary_path, scan_target);
    std::fs::write(path, xml)?;

    // Best-effort unload — ignore errors when the agent isn't already
    // loaded. Then load (or reload) the new plist.
    let _ = Command::new("launchctl").arg("unload").arg(path).output();
    let out = Command::new("launchctl").arg("load").arg(path).output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(WardError::Backup(format!(
            "launchctl load failed ({}): {stderr}",
            path.display()
        )));
    }
    Ok(())
}

/// Unload the agent and delete the plist file. Tolerates an
/// already-uninstalled state.
pub fn remove() -> Result<(), WardError> {
    let path = plist_path();
    let _ = Command::new("launchctl").arg("unload").arg(&path).output();
    match std::fs::remove_file(&path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Test-only remove at an arbitrary root.
#[cfg(test)]
pub fn remove_at(path: &Path) -> Result<(), WardError> {
    let _ = Command::new("launchctl").arg("unload").arg(path).output();
    match std::fs::remove_file(path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// What does the system think is installed? Combines the on-disk
/// plist check with a `launchctl list` lookup. We default to
/// `NotInstalled` whenever either signal says no — the goal is to
/// never report `Installed` falsely.
pub fn status() -> SchedulerStatus {
    let path = plist_path();
    let (on_disk, in_launchd) = (path.exists(), launchctl_has_label());
    match (on_disk, in_launchd) {
        (true, true) => {
            let interval = read_start_interval(&path).unwrap_or(MIN_INTERVAL_SECONDS);
            SchedulerStatus::Installed { interval_seconds: interval }
        }
        (false, false) => SchedulerStatus::NotInstalled,
        (true, false) => SchedulerStatus::Error(
            "plist present but launchd does not know about the agent".into(),
        ),
        (false, true) => SchedulerStatus::Error(
            "launchd knows about the agent but plist is missing".into(),
        ),
    }
}

/// Test-only status: caller supplies the plist path to check + an
/// optional `launchctl list` override (used by tests to simulate the
/// launchd registry without touching the real daemon).
#[cfg(test)]
pub fn status_at(path: &Path, launchctl_override: Option<bool>) -> SchedulerStatus {
    let on_disk = path.exists();
    let in_launchd = match launchctl_override {
        Some(b) => b,
        None => launchctl_has_label(),
    };
    match (on_disk, in_launchd) {
        (true, true) => {
            let interval = read_start_interval(path).unwrap_or(MIN_INTERVAL_SECONDS);
            SchedulerStatus::Installed { interval_seconds: interval }
        }
        (false, false) => SchedulerStatus::NotInstalled,
        (true, false) => SchedulerStatus::Error(
            "plist present but launchd does not know about the agent".into(),
        ),
        (false, true) => SchedulerStatus::Error(
            "launchd knows about the agent but plist is missing".into(),
        ),
    }
}

fn launchctl_has_label() -> bool {
    // `launchctl list <label>` — exit 0 means present, non-zero means
    // absent. We swallow stderr to keep the call quiet in production
    // logs.
    Command::new("launchctl")
        .arg("list")
        .arg(SCHEDULER_LABEL)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parse `<key>StartInterval</key><integer>N</integer>` from the
/// plist. Best-effort — returns `None` if the key isn't present or
/// the value isn't an integer.
fn read_start_interval(path: &Path) -> Option<u32> {
    let s = std::fs::read_to_string(path).ok()?;
    let needle = "<key>StartInterval</key>";
    let after = s.find(needle)? + needle.len();
    let tail = &s[after..];
    let open = tail.find("<integer>")? + "<integer>".len();
    let close = tail[open..].find("</integer>")? + open;
    tail[open..close].trim().parse::<u32>().ok()
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Sanity-check the constants — these are part of the
    /// on-disk contract with installed agents.
    #[test]
    fn label_matches_bundle() {
        assert_eq!(SCHEDULER_LABEL, "dev.balakumar.ward.backup");
        assert!(SCHEDULER_LABEL.starts_with(BUNDLE_PREFIX));
        assert_eq!(PLIST_FILENAME, format!("{SCHEDULER_LABEL}.plist"));
    }

    #[test]
    fn plist_path_under_users_home() {
        let p = plist_path();
        // Either ~/Library/LaunchAgents/<file> or "/" fallback for
        // sandboxed homes with no $HOME. Both end with the filename.
        assert_eq!(p.file_name().and_then(|n| n.to_str()), Some(PLIST_FILENAME));
        assert!(p.to_string_lossy().contains("LaunchAgents"));
    }

    #[test]
    fn validate_interval_rejects_too_low() {
        assert!(matches!(validate_interval(0), Err(WardError::Backup(_))));
        assert!(matches!(validate_interval(60), Err(WardError::Backup(_))));
        assert!(matches!(validate_interval(299), Err(WardError::Backup(_))));
    }

    #[test]
    fn validate_interval_rejects_too_high() {
        assert!(matches!(validate_interval(86_401), Err(WardError::Backup(_))));
        assert!(matches!(validate_interval(120_000), Err(WardError::Backup(_))));
    }

    #[test]
    fn validate_interval_accepts_band() {
        for n in [300u32, 600, 1800, 3600, 7200, 86_400] {
            assert!(validate_interval(n).is_ok(), "interval {n}s must validate");
        }
    }

    #[test]
    fn plist_content_has_expected_keys() {
        let binary = Path::new("/Applications/Ward.app/Contents/MacOS/ward");
        let target = Path::new("/Users/x/.claude");
        let xml = plist_content(900, binary, target);

        // Required entries.
        assert!(xml.contains("<key>Label</key>"));
        assert!(xml.contains(SCHEDULER_LABEL));
        assert!(xml.contains("<key>ProgramArguments</key>"));
        assert!(xml.contains("<key>StartInterval</key>"));
        assert!(xml.contains("<integer>900</integer>"));
        assert!(xml.contains("<key>RunAtLoad</key>"));
        assert!(xml.contains("<false/>"));
        assert!(xml.contains(binary.to_string_lossy().as_ref()));
        assert!(xml.contains("--backup-once"));
        assert!(xml.contains(target.to_string_lossy().as_ref()));
        // Logging path under the backup dir.
        assert!(xml.contains("backup.log"));
        // Validates only — content still refers to real `$HOME/.ward-backups`.
        let home = dirs::home_dir().unwrap();
        assert!(xml.contains(&format!("{}/.ward-backups/backup.log", home.display())));
    }

    #[test]
    fn plist_content_panics_on_invalid_interval() {
        let binary = Path::new("/a/b");
        let target = Path::new("/c/d");
        // The pure plist_content helper asserts, while install() does
        // the proper Result-style validation. Callers always go
        // through install() — so the panic is defensive only.
        let result = std::panic::catch_unwind(|| plist_content(60, binary, target));
        assert!(result.is_err(), "invalid interval should panic in plist_content");
    }

    #[test]
    fn shell_escape_neutralizes_xml_chars() {
        assert_eq!(shell_escape("a&b"), "a&amp;b");
        assert_eq!(shell_escape("a<b>c"), "a&lt;b&gt;c");
        assert_eq!(shell_escape("a\nb"), "ab"); // control chars dropped
        assert_eq!(shell_escape("plain"), "plain");
    }

    #[test]
    fn install_writes_plist_at_expected_path() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("fake/ward");
        let target = root.path().join("home/.claude");
        // Fake binary — we won't try to launch it; the launchctl
        // round-trip is what matters for the write half.
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, "#!/bin/sh\nexit 0\n").unwrap();
        let mut perms = fs::metadata(&binary).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
        }
        fs::set_permissions(&binary, perms).unwrap();

        // Use install_at which won't actually hit the real launchctl
        // on the test bin (we'd need launchd available, which it is
        // on macOS, so we just check the plist file exists).
        let res = install_at(
            root.path(),
            900,
            &binary,
            &target,
        );

        let plist = plist_path_in(root.path());
        match res {
            Ok(_) => {
                assert!(plist.exists(), "plist should exist after install");
                let xml = fs::read_to_string(&plist).unwrap();
                assert!(xml.contains("StartInterval"));
                assert!(xml.contains("900"));
            }
            Err(WardError::Backup(msg)) if msg.contains("launchctl") => {
                // launchd might be unavailable in the sandbox; fall
                // back to checking the plist file was written even if
                // the load step failed.
                assert!(plist.exists(), "plist should still be on disk");
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn remove_clears_plist_and_tolerates_missing() {
        let root = tempfile::tempdir().unwrap();
        let plist = plist_path_in(root.path());
        fs::create_dir_all(plist.parent().unwrap()).unwrap();
        fs::write(&plist, "<plist/>").unwrap();
        assert!(plist.exists());

        // First remove deletes.
        remove_at(&plist).unwrap();
        assert!(!plist.exists());

        // Second remove is a no-op.
        remove_at(&plist).unwrap();
    }

    #[test]
    fn status_reports_installed_only_when_both_signals_agree() {
        let root = tempfile::tempdir().unwrap();
        let plist = plist_path_in(root.path());
        fs::create_dir_all(plist.parent().unwrap()).unwrap();
        fs::write(
            &plist,
            plist_content(1800, Path::new("/ward"), Path::new("/target")),
        )
        .unwrap();

        // Disk + launchd ⇒ installed.
        assert!(matches!(
            status_at(&plist, Some(true)),
            SchedulerStatus::Installed { interval_seconds: 1800 }
        ));
        // Disk only ⇒ error (split brain).
        assert!(matches!(
            status_at(&plist, Some(false)),
            SchedulerStatus::Error(_)
        ));
        // launchd only ⇒ error (without disk).
        fs::remove_file(&plist).unwrap();
        assert!(matches!(
            status_at(&plist, Some(true)),
            SchedulerStatus::Error(_)
        ));
        // Neither ⇒ not installed.
        assert!(matches!(
            status_at(&plist, Some(false)),
            SchedulerStatus::NotInstalled
        ));
    }

    #[test]
    fn status_parses_interval_from_plist() {
        let root = tempfile::tempdir().unwrap();
        let plist = plist_path_in(root.path());
        fs::create_dir_all(plist.parent().unwrap()).unwrap();
        fs::write(
            &plist,
            plist_content(4500, Path::new("/ward"), Path::new("/t")),
        )
        .unwrap();
        let s = status_at(&plist, Some(true));
        match s {
            SchedulerStatus::Installed { interval_seconds } => {
                assert_eq!(interval_seconds, 4500);
            }
            other => panic!("expected Installed, got {other:?}"),
        }
    }
}
