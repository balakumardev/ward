# Ward Plan 08 — Backup Center: git + launchd scheduler (High-Level)

> High-level plan. Use **superpowers:subagent-driven-development**, TDD, commit per task. CCO reference (read-only): `/Users/balakumar/personal/claude-code-organizer`.

**Goal:** Export config → git **commit** → **push**; plus **scheduled** backups via launchd.

**Builds on:** `commands.rs`, scan/export.

**Files:**
- Create `src-tauri/src/backup/git.rs`: init / set-remote / add / commit / push (shell out to `git`, like CCO — or `git2` crate). Backup target dir **`~/.ward-backups/`** (git repo + `backup.log`).
- Create `src-tauri/src/backup/scheduler.rs`: **launchd** plist install/remove/status at `~/Library/LaunchAgents/dev.balakumar.ward.backup.plist` + `launchctl load/unload/list`. (macOS only — **drop CCO's systemd branch.**)
- Modify `src-tauri/src/commands.rs`: `backup_status`, `backup_run` (export→commit→push), `backup_sync` (commit→push), `backup_scheduler_install`, `backup_remote`.
- Create frontend `src/modes/Backups.tsx`: status, Run / Sync, remote-URL field, schedule toggle + interval.

**Task checklist:**
- [ ] Export scanned config into `~/.ward-backups/` (mirror layout).
- [ ] git init / identity fallback / commit / push.
- [ ] launchd plist install/remove/status.
- [ ] Backup commands + `modes/Backups.tsx` UI.

**CCO parity refs:** `src/backup-git.mjs`, `src/backup-scheduler.mjs` (**launchd branch only**), `src/server.mjs` `/api/backup/*` routes.

**Tests:** export produces expected tree; commit creates a commit; scheduler install writes a valid plist + `launchctl` succeeds; status reflects installed/removed.

**Gotchas:** backup dir is `~/.ward-backups` (not CCO's `~/.claude-backups`); plist **label/path** must match `dev.balakumar.ward`; set a git identity fallback if none configured; **push is a network action — never auto-push without explicit user action.**
