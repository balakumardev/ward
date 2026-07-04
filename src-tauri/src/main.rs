// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use ward_lib::cli::{self, CliArgs};

fn main() {
    // Plan 10 — clap-derived CLI dispatch.
    //
    // Headless subcommands (--scan, --security-scan, --backup-once,
    // --mcp) print JSON to stdout and exit before the GUI builder
    // starts. The legacy --backup-once argv-style path is preserved
    // inside `cli::dispatch` for compatibility with the launchd agent.
    let args: CliArgs = match CliArgs::try_parse() {
        Ok(a) => a,
        Err(e) => {
            // clap's own error formatting goes to stderr; honor its
            // requested exit code (2 for usage errors).
            let _ = e.print();
            std::process::exit(2);
        }
    };

    if cli::is_headless(&args) {
        std::process::exit(cli::dispatch(&args));
    }

    ward_lib::run()
}