// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Plan 08 — `ward --backup-once <harness_id>` is invoked headlessly
    // by the launchd scheduler. It runs the same backup pipeline as
    // the UI's "Run backup" button (`backup_run` + `backup_sync`)
    // and exits. The push step is intentionally NEVER part of this
    // path — push is a network action gated to user clicks.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--backup-once") {
        std::process::exit(ward_lib::run_backup_once(&args));
    }

    ward_lib::run()
}
