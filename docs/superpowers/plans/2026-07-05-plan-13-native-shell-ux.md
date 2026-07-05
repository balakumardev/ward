# Plan 13 — Native Shell UX (close-to-tray, launch-at-login, app icon) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Ward behave like a native macOS menu-bar app — closing the window hides it to the tray (the agent keeps running), it launches at login by default (once, respecting a later opt-out), it can start hidden into the menu bar, and it ships the approved "shield + circuit grid" app icon plus a clean monochrome tray glyph.

**Architecture:** All backend (Rust) + icon assets. A new `native/lifecycle.rs` holds the quit-vs-hide decision behind a shared atomic; `lib.rs`'s builder gains an `on_window_event` close interceptor and switches to the `build(...)?.run(callback)` form so macOS `ExitRequested` (⌘Q) is caught. A new `native/autostart.rs` wraps `tauri-plugin-autostart` with a first-run sentinel gate under `~/.ward/`. A new `--start-hidden` CLI flag (parsed by the existing strict clap surface) lets the LaunchAgent boot Ward into the tray. Icons are regenerated from a masked 1024² source via `tauri icon`.

**Tech Stack:** Rust, Tauri v2 (`tray-icon` + `image-png` features), `tauri-plugin-autostart`, `clap` (derive), `dirs`, Python 3 + Pillow (one-off icon asset prep).

## Global Constraints

- **Tauri v2 only.** `invoke` from `@tauri-apps/api/core`; commands registered in `src-tauri/src/lib.rs` via `tauri::generate_handler!`. JS camelCase args → Rust snake_case (automatic).
- **Errors:** `thiserror` + the existing manual `impl serde::Serialize for WardError` (`#[serde(tag="kind", content="message")]`, `rename_all="camelCase"`). New error cases extend `WardError`.
- **Naming (do not rename):** `WardError`, `Ctx`, `Registry`, `ClaudeAdapter`, `run_scan`, `run()`, `Plan10Tray`, `Plan10Watcher`, `tray::setup`, `tray::update_badge`, `tray::format_tooltip`.
- **Always commit `Cargo.lock`** (reproducible Tauri builds).
- **One commit per task**, conventional prefix (`feat:`/`chore:`/`refactor:`/`test:`).
- **Never auto-push; never auto-run backups** — network/external actions pause for the user.
- **No stubs / TODOs / placeholders.** Every function fully implemented and wired.
- **macOS hands-on:** the actual login-item behavior, close-to-tray, and start-hidden are verified by the user in `npm run tauri dev` / a built bundle (documented at the end). `tauri-driver` E2E cannot run on macOS.
- **Ward state dir is `~/.ward/`.** Never write to `~/.claude` or `~/.codex`.
- **`docs/` is force-added** (`git add -f`) because the global gitignore ignores it.

---

## Task 1: `--start-hidden` CLI flag + hide-at-startup

Add a GUI-only flag so the launch-at-login LaunchAgent can boot Ward straight into the menu bar. The existing clap surface is strict (`unknown_flag_errors` test), so the flag must be a known field or startup would `exit(2)`.

**Files:**
- Modify: `src-tauri/src/cli.rs` (add `start_hidden` field to `CliArgs`; update the `parses_default_args` struct literal)
- Modify: `src-tauri/src/lib.rs` (hide the main window in `setup()` when `--start-hidden` is present)
- Test: `src-tauri/src/cli.rs` (unit tests, same file)

**Interfaces:**
- Consumes: nothing new.
- Produces: `CliArgs.start_hidden: bool` (default `false`, not headless); a `setup()` branch that hides `main` when `std::env::args()` contains `--start-hidden`. Task 2 registers the LaunchAgent with this exact arg string.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/cli.rs`:

```rust
    #[test]
    fn parses_start_hidden_flag() {
        let args = parse_from(["ward", "--start-hidden"]).unwrap();
        assert!(args.start_hidden);
        // GUI-only: must NOT be treated as a headless subcommand.
        assert!(!is_headless(&args));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib cli::tests::parses_start_hidden_flag`
Expected: FAIL — `no field 'start_hidden' on type 'CliArgs'` (compile error).

- [ ] **Step 3: Add the field to `CliArgs`**

In `src-tauri/src/cli.rs`, add this field to the `CliArgs` struct (after the `mcp` field, before the closing brace):

```rust
    /// GUI-only: start hidden to the menu bar. Passed by the
    /// launch-at-login LaunchAgent (Plan 13) so Ward boots into the
    /// tray without stealing focus. NOT a headless subcommand.
    #[arg(long)]
    pub start_hidden: bool,
```

- [ ] **Step 4: Fix the existing full-struct test**

The `parses_default_args` test builds a full `CliArgs { .. }` literal and will no longer compile. Update it to include the new field:

```rust
    #[test]
    fn parses_default_args() {
        let args = parse_from(["ward"]).unwrap();
        assert_eq!(args, CliArgs {
            scan: false,
            harness: "claude".to_string(),
            security_scan: false,
            backup_once: None,
            mcp: false,
            start_hidden: false,
        });
    }
```

- [ ] **Step 5: Hide the main window at startup when the flag is present**

In `src-tauri/src/lib.rs`, inside the `.setup(|app| { .. })` closure, immediately **after** the tray-setup `match` block (the one that `manage`s `Plan10Tray`) and before the watcher block, add:

```rust
            // Launch-at-login UX (Plan 13): when the LaunchAgent starts
            // Ward with `--start-hidden`, hide the main window so it boots
            // into the menu bar instead of stealing focus. Reopen via the
            // tray "Open Ward" menu item.
            if std::env::args().any(|a| a == "--start-hidden") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
```

(`get_webview_window` is already available via the `use tauri::{Emitter, Listener, Manager};` import at the top of `lib.rs`.)

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib cli::tests`
Expected: PASS (all cli tests, including `parses_start_hidden_flag`, `parses_default_args`, and `unknown_flag_errors`).

- [ ] **Step 7: Typecheck the whole crate**

Run: `cd src-tauri && cargo check`
Expected: no errors.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/cli.rs src-tauri/src/lib.rs
git commit -m "feat(plan13): --start-hidden flag + boot-to-tray for launch-at-login"
```

---

## Task 2: Launch-at-login (autostart)

Add `tauri-plugin-autostart`, wrap it in `native/autostart.rs` behind `WardError`, register a per-user macOS LaunchAgent, and enable it **once** on first run gated by a `~/.ward/first-run` sentinel so a later user opt-out sticks. Expose `autostart_status` / `autostart_set` commands (consumed by the Plan 15 popover toggle).

**Files:**
- Modify: `src-tauri/Cargo.toml` (add dependency)
- Modify: `src-tauri/src/error.rs` (add `WardError::Autostart` variant + `ErrorKind` arm)
- Create: `src-tauri/src/native/autostart.rs`
- Modify: `src-tauri/src/native/mod.rs` (`pub mod autostart;`)
- Modify: `src-tauri/src/commands.rs` (add `autostart_status`, `autostart_set`)
- Modify: `src-tauri/src/lib.rs` (register plugin, call `enable_on_first_run`, register commands)
- Test: `src-tauri/src/native/autostart.rs` (unit tests, same file)

**Interfaces:**
- Consumes: `WardError` (extended here).
- Produces:
  - `native::autostart::should_enable_on_first_run(sentinel_exists: bool, currently_enabled: bool) -> bool`
  - `native::autostart::first_run_sentinel_path() -> Result<PathBuf, WardError>`
  - `native::autostart::mark_first_run_done() -> Result<(), WardError>`
  - `native::autostart::status<R: Runtime>(&AppHandle<R>) -> Result<bool, WardError>`
  - `native::autostart::set<R: Runtime>(&AppHandle<R>, enabled: bool) -> Result<(), WardError>`
  - `native::autostart::enable_on_first_run<R: Runtime>(&AppHandle<R>)`
  - Tauri commands `autostart_status(app) -> Result<bool, WardError>`, `autostart_set(app, enabled: bool) -> Result<(), WardError>` (Plan 15 wraps these in `api.ts` as `autostartStatus()` / `autostartSet(enabled)`).

- [ ] **Step 1: Add the dependency**

In `src-tauri/Cargo.toml`, under `[dependencies]`, after the Plan 10 block, add:

```toml
# Plan 13 — launch-at-login (per-user LaunchAgent on macOS)
tauri-plugin-autostart = "2"
```

- [ ] **Step 2: Add the `Autostart` error variant**

In `src-tauri/src/error.rs`:

Add to the `WardError` enum (after the `Backup` variant):

```rust
    #[error("autostart error: {0}")]
    Autostart(String),
```

Add to the private `ErrorKind` enum (after `Backup(String)`):

```rust
    Autostart(String),
```

Add to the `match self` in `impl serde::Serialize for WardError` (after the `Backup` arm):

```rust
            WardError::Autostart(_) => ErrorKind::Autostart(message),
```

- [ ] **Step 3: Write the failing tests for the autostart module**

Create `src-tauri/src/native/autostart.rs` with ONLY the tests + imports first (implementation stubbed enough to compile fails on purpose is not allowed — instead write the real signatures in Step 5; here we write the test file section). Put this test module at the bottom of the new file, but to see it fail we first need the module to exist. Create the file with the full contents from Step 5 **except** replace the function bodies you haven't written — no: write the tests now and the impl in Step 5. To keep TDD honest, create the file now containing just the pure helpers' tests and empty impls that fail:

```rust
//! Plan 13 — launch-at-login (autostart) helpers. (WIP — filled in Step 5.)
use std::path::PathBuf;
use crate::error::WardError;

pub fn should_enable_on_first_run(_sentinel_exists: bool, _currently_enabled: bool) -> bool {
    unimplemented!()
}
pub fn first_run_sentinel_path() -> Result<PathBuf, WardError> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enables_only_on_true_first_run() {
        assert!(should_enable_on_first_run(false, false));   // first run, not enabled → enable
        assert!(!should_enable_on_first_run(true, false));   // already ran once → leave alone
        assert!(!should_enable_on_first_run(false, true));   // first run but already enabled → skip
        assert!(!should_enable_on_first_run(true, true));    // ran + enabled → skip
    }

    #[test]
    fn sentinel_path_is_under_dot_ward() {
        let p = first_run_sentinel_path().unwrap();
        assert!(p.ends_with(".ward/first-run"), "got {p:?}");
    }
}
```

Add `pub mod autostart;` to `src-tauri/src/native/mod.rs` (after `pub mod tray;`, keeping alphabetical-ish order is fine):

```rust
pub mod autostart;
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib native::autostart::tests`
Expected: FAIL — both tests panic with `not implemented` (`unimplemented!()`).

- [ ] **Step 5: Write the real implementation**

Replace the entire contents of `src-tauri/src/native/autostart.rs` with:

```rust
//! Plan 13 — launch-at-login (autostart) helpers.
//!
//! Wraps `tauri-plugin-autostart`'s `ManagerExt` so the rest of the app
//! deals in plain `Result<_, WardError>`. The plugin registers a per-user
//! macOS LaunchAgent — distinct from the Plan 08 backup LaunchAgent
//! (`dev.balakumar.ward.backup`), so the two never collide.
//!
//! First-run policy: Ward enables launch-at-login ONCE, the first time it
//! starts, gated by a `~/.ward/first-run` sentinel. After that we never
//! re-enable, so a user who turns it off stays off.

use std::path::PathBuf;

use tauri::{AppHandle, Runtime};
use tauri_plugin_autostart::ManagerExt;

use crate::error::WardError;

/// Path to the first-run sentinel: `~/.ward/first-run`.
pub fn first_run_sentinel_path() -> Result<PathBuf, WardError> {
    let home = dirs::home_dir().ok_or_else(|| WardError::NotFound("home directory".into()))?;
    Ok(home.join(".ward").join("first-run"))
}

/// Pure decision: auto-enable launch-at-login only on the very first run
/// (no sentinel yet) AND only if it isn't already enabled.
pub fn should_enable_on_first_run(sentinel_exists: bool, currently_enabled: bool) -> bool {
    !sentinel_exists && !currently_enabled
}

/// Create `~/.ward/` (if needed) and write the sentinel so we never
/// auto-enable again.
pub fn mark_first_run_done() -> Result<(), WardError> {
    let path = first_run_sentinel_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, b"1")?;
    Ok(())
}

/// Is launch-at-login currently enabled?
pub fn status<R: Runtime>(app: &AppHandle<R>) -> Result<bool, WardError> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| WardError::Autostart(format!("is_enabled: {e}")))
}

/// Enable or disable launch-at-login.
pub fn set<R: Runtime>(app: &AppHandle<R>, enabled: bool) -> Result<(), WardError> {
    let mgr = app.autolaunch();
    let res = if enabled { mgr.enable() } else { mgr.disable() };
    res.map_err(|e| WardError::Autostart(format!("set({enabled}): {e}")))
}

/// First-run policy: enable launch-at-login once, then mark done. Errors
/// are logged, never propagated — autostart must never block startup.
pub fn enable_on_first_run<R: Runtime>(app: &AppHandle<R>) {
    let sentinel_exists = first_run_sentinel_path().map(|p| p.exists()).unwrap_or(true);
    let currently_enabled = status(app).unwrap_or(false);
    if should_enable_on_first_run(sentinel_exists, currently_enabled) {
        if let Err(e) = set(app, true) {
            eprintln!("ward: autostart first-run enable failed: {e}");
        }
    }
    if let Err(e) = mark_first_run_done() {
        eprintln!("ward: autostart mark first-run failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enables_only_on_true_first_run() {
        assert!(should_enable_on_first_run(false, false));
        assert!(!should_enable_on_first_run(true, false));
        assert!(!should_enable_on_first_run(false, true));
        assert!(!should_enable_on_first_run(true, true));
    }

    #[test]
    fn sentinel_path_is_under_dot_ward() {
        let p = first_run_sentinel_path().unwrap();
        assert!(p.ends_with(".ward/first-run"), "got {p:?}");
    }
}
```

- [ ] **Step 6: Run the module tests to verify they pass**

Run: `cd src-tauri && cargo test --lib native::autostart::tests`
Expected: PASS (both tests).

- [ ] **Step 7: Add the Tauri commands**

In `src-tauri/src/commands.rs`, add these two commands (place them near the end of the file, after `backup_scheduler_remove`; they need no other imports beyond `crate::error::WardError`, already imported):

```rust
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
```

- [ ] **Step 8: Register the plugin, first-run gate, and commands in `lib.rs`**

In `src-tauri/src/lib.rs`:

(a) Register the plugin — add after the `.plugin(tauri_plugin_notification::init())` line:

```rust
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--start-hidden"]),
        ))
```

(b) Enable on first run — inside `.setup(|app| { .. })`, after the `--start-hidden` block from Task 1, add:

```rust
            // Launch-at-login (Plan 13): enable once on first run, gated by
            // a ~/.ward/first-run sentinel so a later opt-out persists.
            crate::native::autostart::enable_on_first_run(app.handle());
```

(c) Register the commands — inside `tauri::generate_handler![ .. ]`, add after `commands::backup_scheduler_remove` (add a trailing comma to the current last entry):

```rust
            commands::backup_scheduler_remove,
            commands::autostart_status,
            commands::autostart_set
```

- [ ] **Step 9: Build and run the full test suite**

Run: `cd src-tauri && cargo check && cargo test --lib`
Expected: `cargo check` clean; all existing + new lib tests PASS (including `error::tests` and `native::autostart::tests`).

- [ ] **Step 10: Commit (including Cargo.lock)**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/error.rs \
        src-tauri/src/native/autostart.rs src-tauri/src/native/mod.rs \
        src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(plan13): launch-at-login via tauri-plugin-autostart with first-run gate"
```

---

## Task 3: Close-to-tray

Closing the main window (red button / ⌘W) hides it to the menu bar instead of quitting; the tray + watcher keep running. Genuine quits (tray **Quit**, ⌘Q) still terminate. A shared atomic distinguishes the two.

**Files:**
- Create: `src-tauri/src/native/lifecycle.rs`
- Modify: `src-tauri/src/native/mod.rs` (`pub mod lifecycle;`)
- Modify: `src-tauri/src/lib.rs` (tray "quit" marks quitting; add `on_window_event`; switch tail to `build(..)?.run(callback)`)
- Test: `src-tauri/src/native/lifecycle.rs` (unit tests, same file)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `native::lifecycle::QUITTING: AtomicBool`
  - `native::lifecycle::mark_quitting()`
  - `native::lifecycle::is_quitting() -> bool`
  - `native::lifecycle::should_hide_on_close(quitting: bool) -> bool`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/native/lifecycle.rs`:

```rust
//! Plan 13 — app lifecycle: close-to-tray vs. real quit. (WIP.)
pub fn should_hide_on_close(_quitting: bool) -> bool {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_when_not_quitting() {
        assert!(should_hide_on_close(false));
    }

    #[test]
    fn does_not_hide_when_quitting() {
        assert!(!should_hide_on_close(true));
    }
}
```

Add `pub mod lifecycle;` to `src-tauri/src/native/mod.rs`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib native::lifecycle::tests`
Expected: FAIL — both panic with `not implemented`.

- [ ] **Step 3: Write the real implementation**

Replace the entire contents of `src-tauri/src/native/lifecycle.rs` with:

```rust
//! Plan 13 — app lifecycle: close-to-tray vs. real quit.
//!
//! `QUITTING` is set true only by a genuine quit path (the tray "Quit"
//! menu item, or macOS `ExitRequested` from ⌘Q). While it is false, a
//! window `CloseRequested` (red button / ⌘W) hides the window to the
//! menu bar instead of letting it close — so the tray + fs-watcher keep
//! the background agent alive.

use std::sync::atomic::{AtomicBool, Ordering};

/// True while a genuine quit is in progress.
pub static QUITTING: AtomicBool = AtomicBool::new(false);

/// Mark that a genuine quit is in progress.
pub fn mark_quitting() {
    QUITTING.store(true, Ordering::SeqCst);
}

/// Is a genuine quit in progress?
pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

/// Should a window `CloseRequested` hide-to-tray (true) rather than
/// proceed as a real close (false)? Hide unless we're genuinely quitting.
pub fn should_hide_on_close(quitting: bool) -> bool {
    !quitting
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_when_not_quitting() {
        assert!(should_hide_on_close(false));
    }

    #[test]
    fn does_not_hide_when_quitting() {
        assert!(!should_hide_on_close(true));
    }

    #[test]
    fn mark_and_read_quitting_roundtrip() {
        QUITTING.store(false, Ordering::SeqCst);
        assert!(!is_quitting());
        mark_quitting();
        assert!(is_quitting());
        QUITTING.store(false, Ordering::SeqCst); // reset so other tests see a clean flag
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib native::lifecycle::tests`
Expected: PASS (all three).

- [ ] **Step 5: Mark quitting from the tray "Quit" action**

In `src-tauri/src/lib.rs`, inside the `app.listen("tray_action", ..)` handler, change the `"quit"` arm from:

```rust
                    "quit" => {
                        app_handle.exit(0);
                    }
```

to:

```rust
                    "quit" => {
                        crate::native::lifecycle::mark_quitting();
                        app_handle.exit(0);
                    }
```

- [ ] **Step 6: Add the close interceptor and switch to `build(..)?.run(callback)`**

In `src-tauri/src/lib.rs`, replace the tail of `run()` — from `.invoke_handler(...)` through the final `.expect(...)` — as follows.

Insert `.on_window_event(...)` immediately **before** `.invoke_handler(`:

```rust
        .on_window_event(|window, event| {
            // Close-to-tray (Plan 13): the red button / ⌘W hides the
            // window to the menu bar; only a genuine quit closes it.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if crate::native::lifecycle::should_hide_on_close(
                    crate::native::lifecycle::is_quitting(),
                ) {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
```

Then replace the final two lines:

```rust
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
```

with the build + run-callback form (so macOS `ExitRequested` from ⌘Q is caught and marks quitting before any teardown `CloseRequested` fires):

```rust
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                crate::native::lifecycle::mark_quitting();
            }
        });
```

- [ ] **Step 7: Typecheck + full lib tests**

Run: `cd src-tauri && cargo check && cargo test --lib`
Expected: clean check; all lib tests PASS.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/native/lifecycle.rs src-tauri/src/native/mod.rs src-tauri/src/lib.rs
git commit -m "feat(plan13): close-to-tray — hide main window on close, keep agent alive"
```

---

## Task 4: App icon (concept A)

Replace the placeholder Tauri icons with the approved "shield + circuit grid" artwork. The generated source has non-transparent corners outside its squircle; mask them, then regenerate every bundle size with `tauri icon`.

**Files:**
- Create: `src-tauri/icons/icon-source.png` (masked 1024² source)
- Regenerate (overwrite): `src-tauri/icons/32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns`, `icon.ico`, plus the Windows/Store PNGs `tauri icon` emits
- No config change: `tauri.conf.json` already references the 5 macOS/Windows icon paths.

**Interfaces:**
- Consumes: the approved concept-A PNG at `/private/tmp/claude-501/-Users-balakumar-personal-ward/cce4244e-ba0f-4bc3-85fe-8ed4e1ef1b75/scratchpad/ward-icon-A-shield.png`.
- Produces: regenerated icon binaries in `src-tauri/icons/`.

- [ ] **Step 1: Ensure Pillow is available**

Run: `python3 -c "import PIL; print(PIL.__version__)" || python3 -m pip install --quiet --user Pillow`
Expected: prints a version, or installs Pillow then succeeds on a re-run.

- [ ] **Step 2: Mask the source to a transparent-cornered 1024² squircle**

Run this one-off script (writes the masked source into the repo). Paths are absolute so the Bash cwd reset is irrelevant:

```bash
python3 - <<'PY'
from PIL import Image, ImageDraw
SRC = "/private/tmp/claude-501/-Users-balakumar-personal-ward/cce4244e-ba0f-4bc3-85fe-8ed4e1ef1b75/scratchpad/ward-icon-A-shield.png"
OUT = "/Users/balakumar/personal/ward/src-tauri/icons/icon-source.png"
src = Image.open(SRC).convert("RGBA").resize((1024, 1024), Image.LANCZOS)
mask = Image.new("L", (1024, 1024), 0)
ImageDraw.Draw(mask).rounded_rectangle([0, 0, 1023, 1023], radius=230, fill=255)
out = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
out.paste(src, (0, 0), mask)
out.save(OUT)
print("wrote", OUT)
PY
```

Expected: `wrote /Users/balakumar/personal/ward/src-tauri/icons/icon-source.png`.
(If the corner radius leaves white slivers or clips the artwork, adjust `radius` up/down ~40px and re-run — the artwork is a full-bleed squircle so ~230 should match.)

- [ ] **Step 3: Regenerate all icon sizes**

Run: `cd /Users/balakumar/personal/ward && npm run tauri icon src-tauri/icons/icon-source.png`
Expected: Tauri prints the sizes it wrote (`32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns`, `icon.ico`, `Square*Logo.png`, `StoreLogo.png`, `icon.png`) into `src-tauri/icons/`.

- [ ] **Step 4: Verify the expected files exist and are non-empty**

Run: `cd /Users/balakumar/personal/ward && for f in 32x32.png 128x128.png 128x128@2x.png icon.icns icon.ico; do test -s "src-tauri/icons/$f" && echo "ok $f" || echo "MISSING $f"; done`
Expected: `ok` for all five.

- [ ] **Step 5: Confirm the app still builds with the new icons**

Run: `cd src-tauri && cargo check`
Expected: no errors (icons are referenced by `tauri.conf.json` at bundle time; `cargo check` validates the crate compiles).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/icons/
git commit -m "feat(plan13): ship Ward app icon (shield + circuit grid)"
```

---

## Task 5: Monochrome tray (menu-bar) glyph

The tray currently reuses the full-color window icon as a template, which renders muddy in the menu bar. Ship a clean black-on-transparent shield template and point the tray at it. macOS treats template images by alpha, tinting to the menu-bar color for light/dark.

**Files:**
- Create: `src-tauri/icons/tray-template.png` (44² black shield, transparent bg)
- Modify: `src-tauri/Cargo.toml` (add `image-png` feature to `tauri`)
- Modify: `src-tauri/src/native/tray.rs` (`setup()` loads the bundled template instead of `default_window_icon`)

**Interfaces:**
- Consumes: nothing new.
- Produces: a tray icon rendered from `icons/tray-template.png`.

- [ ] **Step 1: Draw the tray template glyph**

Run (Pillow from Task 4 is already available):

```bash
python3 - <<'PY'
from PIL import Image, ImageDraw
OUT = "/Users/balakumar/personal/ward/src-tauri/icons/tray-template.png"
S = 176  # supersample, then downscale for crisp anti-aliasing
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
pad = int(S * 0.14)
midx = S // 2
shoulder = int(S * 0.07)
pts = [
    (pad, pad + shoulder),
    (midx, pad),
    (S - pad, pad + shoulder),
    (S - pad, int(S * 0.56)),
    (midx, S - pad),
    (pad, int(S * 0.56)),
]
d.polygon(pts, fill=(0, 0, 0, 255))  # template: black + alpha; macOS tints it
img.resize((44, 44), Image.LANCZOS).save(OUT)
print("wrote", OUT)
PY
```

Expected: `wrote /Users/balakumar/personal/ward/src-tauri/icons/tray-template.png`.

- [ ] **Step 2: Enable PNG decoding in Tauri**

In `src-tauri/Cargo.toml`, change the `tauri` dependency line:

```toml
tauri = { version = "2", features = ["tray-icon"] }
```

to:

```toml
tauri = { version = "2", features = ["tray-icon", "image-png"] }
```

- [ ] **Step 3: Point the tray at the template glyph**

In `src-tauri/src/native/tray.rs`, in `setup()`, replace the icon-loading block:

```rust
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| WardError::NotFound("default tray icon missing".into()))?;
```

with a load of the embedded template (decoded via the `image-png` feature):

```rust
    // Plan 13 — dedicated monochrome template so the menu-bar glyph adapts
    // cleanly to light/dark instead of muddily tinting the color app icon.
    let icon = tauri::image::Image::from_bytes(include_bytes!("../../icons/tray-template.png"))
        .map_err(|e| WardError::NotFound(format!("tray template icon: {e}")))?;
```

(`.icon_as_template(true)` is already set in the builder chain just below, so no other change is needed.)

- [ ] **Step 4: Typecheck (validates the embed path + image API + feature)**

Run: `cd src-tauri && cargo check`
Expected: no errors. (If `Image::from_bytes` is unavailable, confirm the `image-png` feature landed in Step 2.)

- [ ] **Step 5: Run the tray unit tests (unchanged formatters still pass)**

Run: `cd src-tauri && cargo test --lib native::tray::tests`
Expected: PASS (the existing tooltip/badge tests are unaffected).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/icons/tray-template.png src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/native/tray.rs
git commit -m "feat(plan13): monochrome menu-bar tray template glyph"
```

---

## Hands-on verification (user, after all tasks)

`tauri-driver` E2E can't run on macOS, so verify the native behaviors manually in `npm run tauri dev` (a window opens):

1. **Close-to-tray:** click the red close button → window disappears, Ward stays in the menu bar; the tray icon is still there. Click the tray → **Open Ward** → window returns.
2. **Real quit:** tray → **Quit** (and ⌘Q) → Ward fully exits (no lingering menu-bar icon, process gone).
3. **Launch-at-login:** on first launch, check System Settings → General → Login Items → "Open at Login" lists Ward. Toggle it off, relaunch → it stays off (sentinel honored).
4. **Start-hidden:** run `./src-tauri/target/debug/ward --start-hidden` → Ward boots into the menu bar with no window; tray **Open Ward** shows it.
5. **Icons:** the Dock/Finder icon is the shield-on-grid; the menu-bar glyph is a clean monochrome shield that inverts correctly in light vs dark menu bars.

---

## Self-Review

**Spec coverage (Plan 13 slice of `2026-07-05-ward-menu-bar-agent-design.md` §12):**
- close-to-tray → Task 3 ✓
- launch-at-login (first-run gated, toggleable) → Task 2 ✓ (toggle UI is Plan 15, commands produced here)
- app icon → Task 4 ✓; monochrome tray template → Task 5 ✓
- start-hidden login UX (spec §7.2 "boots into the menu bar") → Task 1 ✓
- **Deviation:** the spec's Plan-13 line also lists "tray tooltip + Dock badge from scan." That wiring needs the security-scan result and a frontend call, so it is **moved to Plan 15** (which edits the Security/usage frontend and adds `native_update_status`). `tray::update_badge` / `format_tooltip` already exist and stay untouched here. Noted so Plan 15 picks it up.

**Placeholder scan:** the two `unimplemented!()` bodies are intentional TDD red-state stubs, each removed in the same task's next step (Task 2 Step 5, Task 3 Step 3). No `TODO`/`TBD`/"handle edge cases" remain.

**Type consistency:** `should_enable_on_first_run`, `first_run_sentinel_path`, `mark_first_run_done`, `status`, `set`, `enable_on_first_run` (Task 2) and `mark_quitting`/`is_quitting`/`should_hide_on_close`/`QUITTING` (Task 3) are referenced with identical signatures at every call site in `lib.rs`/`commands.rs`. Command names `autostart_status`/`autostart_set` match between `commands.rs` and the `generate_handler!` registration. Icon filenames match `tauri.conf.json` and the `include_bytes!` path (`../../icons/tray-template.png` from `src/native/tray.rs`).
