# Ward Plan 10 — Native Shell: menu-bar / background scan / notifications / fs-watch (High-Level)

> High-level plan. Use **superpowers:subagent-driven-development**, TDD where possible (native/UI steps are partly manual), commit per task. CCO reference (read-only): `/Users/balakumar/personal/claude-code-organizer` (no equivalent — this is net-new native behavior).

**Goal:** The native superpowers — a **menu-bar agent** (glance + alert), **background scheduled scans**, **native notifications**, and **filesystem watching** for live refresh.

**Builds on:** security scan (Plan 05), `commands.rs`, Tauri shell.

**Files:**
- Create `src-tauri/src/native/tray.rs`: menu-bar tray icon + **glance popover** (top findings, last-scan time, schedule, **Scan now** / **Open**); badge when findings exist.
- Create `src-tauri/src/native/notify.rs`: `tauri-plugin-notification` — fire on **new critical** findings.
- Create `src-tauri/src/native/watch.rs`: `notify` crate watching `~/.claude`, `~/.codex`, project dirs → emit a `config-changed` event (debounced) → UI re-scans.
- Modify `src-tauri/src/main.rs`: add a headless **`--scan` CLI mode** (for the scheduled job).
- Modify `src-tauri/src/lib.rs`: register tray + notification plugin + start watcher.
- Add deps: Tauri `tray-icon` feature, `tauri-plugin-notification`, `notify` crate.
- Frontend: listen for `config-changed` (via `@tauri-apps/api/event`) → refresh; build the menu-bar popover view.

**Task checklist:**
- [ ] Tray icon + glance popover UI + badge.
- [ ] Native notification on new critical finding.
- [ ] fs-watch → `config-changed` event → UI refresh (debounced).
- [ ] Background scheduled scan: **decide** launchd job invoking `ward --scan` vs. in-process timer while running (affects whether scans run when the app is fully quit) — implement the chosen path.

**CCO parity refs:** none for the native shell; reuse Plan 05's `security_scan` core for both the tray and the scheduled job.

**Tests:** watcher emits on a fixture file change; `--scan` CLI exits 0 and writes results; notification fires on a synthetic new-critical (mock). Tray/popover verified manually.

**Gotchas:** tray + popover lifecycle (keep-alive when window closed); notification permission prompt on first run; debounce fs-watch to avoid scan storms; the scheduled-scan-when-quit decision is the key design fork — document what you chose.
