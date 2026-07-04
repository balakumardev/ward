pub mod backup;
pub mod cli;
mod commands;
mod effective;
mod error;
mod fs_utils;
mod harness;
mod model;
pub mod mcp;
pub mod security;
pub mod sessions;
pub mod tokenizer;

use crate::backup::git as git_ops;
use crate::harness::adapters::claude::ClaudeAdapter;
use crate::harness::{framework, Ctx, Registry};

/// Plan 08 — headless backup run triggered by the launchd agent.
///
/// Parses `--backup-once <scan_target>` out of `argv`, runs
/// `backup_run` semantics (export + commit) and exits with the
/// process status. Returns exit code 0 on success, non-zero on
/// failure. Does NOT push — that's gated behind `backup_push`.
pub fn run_backup_once(argv: &[String]) -> i32 {
    let scan_target = match argv.windows(2).find(|w| w[0] == "--backup-once") {
        Some(w) => w[1].clone(),
        None => {
            eprintln!("ward: --backup-once requires <scan_target>");
            return 2;
        }
    };

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!("ward: cannot resolve HOME");
            return 3;
        }
    };
    let bd = match git_ops::backup_dir() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("ward: backup_dir: {e}");
            return 4;
        }
    };
    let repo = git_ops::repo_dir(&bd);

    // Init the repo on first run.
    if let Err(e) = git_ops::init(
        repo,
        git_ops::FALLBACK_USER_NAME,
        git_ops::FALLBACK_USER_EMAIL,
    ) {
        eprintln!("ward: init failed: {e}");
        return 5;
    }
    let _ = git_ops::ensure_identity_or_fallback(
        repo,
        git_ops::FALLBACK_USER_NAME,
        git_ops::FALLBACK_USER_EMAIL,
    );

    // Resolve which harness we're backing up. Today only claude;
    // the launcher forwards the harness id string so Plan 09 (Codex)
    // slots in transparently.
    let harness_id = scan_target.as_str();
    let scan = match harness_id {
        "claude" => {
            let mut r = Registry::new();
            r.register(Box::new(ClaudeAdapter));
            let ctx = Ctx { home: &home, cwd: None };
            match framework::run_scan(&ClaudeAdapter, &ctx) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("ward: scan failed: {e}");
                    return 6;
                }
            }
        }
        other => {
            eprintln!("ward: unknown scan_target: {other}");
            return 7;
        }
    };
    let _ = scan;

    // Export + commit the source tree.
    let source_root = home.join(".claude");
    let report = match git_ops::export_to_repo(&source_root, repo) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ward: export failed: {e}");
            return 8;
        }
    };
    if report.files_copied > 0 {
        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let msg = format!("backup: ward (claude) {}", ts);
        if let Err(e) = git_ops::commit(repo, &msg) {
            eprintln!("ward: commit failed: {e}");
            return 9;
        }
    }
    0
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::scan,
            commands::read_file_content,
            commands::list_destinations,
            commands::move_item,
            commands::delete_item,
            commands::restore,
            commands::save_file,
            commands::bulk,
            commands::bulk_restore,
            commands::mcp_get_disabled,
            commands::mcp_set_disabled,
            commands::mcp_get_policy,
            commands::mcp_set_policy,
            commands::mcp_check_policy,
            commands::security_scan,
            commands::security_baseline_check,
            commands::security_baseline_accept,
            commands::context_budget,
            commands::session_preview,
            commands::session_cost,
            commands::session_distill,
            commands::session_trim,
            commands::backup_status,
            commands::backup_run,
            commands::backup_sync,
            commands::backup_push,
            commands::backup_set_remote,
            commands::backup_scheduler_install,
            commands::backup_scheduler_remove
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}