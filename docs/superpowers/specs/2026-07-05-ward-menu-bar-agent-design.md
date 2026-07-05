# Ward — Menu-bar Agent Completion (design spec)

- **Date:** 2026-07-05
- **Status:** Approved for planning (brainstorming complete)
- **Depends on:** Plan 01 (foundation), Plan 05 (security scan), Plan 07 (sessions parser + cost), Plan 09 (Codex adapter), Plan 10 (native shell: tray, fs-watch, `--scan`, notification-dedup)
- **Follow-on plans:** 13 (native shell UX), 14 (usage engine), 15 (glance popover)
- **Supersedes:** nothing — this *completes and extends* the "menu-bar agent" already described in `2026-07-04-ward-native-tauri-design.md` (feature 16, decision line "Menu-bar | Glance + alert").

---

## 1. Summary

Complete Ward's menu-bar agent so it behaves like a first-class macOS menu-bar app:

1. **Close-to-tray** — closing the main window hides it to the menu bar instead of quitting; the background agent (fs-watch, notifications) keeps running.
2. **Launch at login** — a per-user LaunchAgent registered via `tauri-plugin-autostart`, enabled once on first run and toggleable from the popover.
3. **Glance popover** — a small webview anchored under the tray icon showing **Claude & Codex usage** (tokens/cost, percent where available) and **reset countdowns**, the current critical-findings count, and Scan-now / Open / launch-at-login controls.
4. **App icon** — ship the approved "shield + circuit grid" icon (concept A) across all bundle sizes plus a monochrome menu-bar template.

All usage data is derived **locally, with no network calls and no credential reading**, consistent with Ward's hard no-auto-network guardrail.

### Goals

- Real usage + reset-time readout for both harnesses, computed from files already on disk.
- Close-to-tray + launch-at-login that feel native and respect the user's explicit choices.
- A popover that reuses the existing Vite bundle, tokens, and event plumbing (`config-changed`, `scan-now`).
- Golden-tested usage math ported faithfully from the MIT `ccusage`/`ccstat` lineage, with `NOTICE` attribution.

### Non-goals (explicitly deferred)

- **Live Anthropic usage API / true Claude plan-%.** Requires an undocumented endpoint + OAuth credentials + background network. Out of scope; the architecture leaves a clean seam (`UsageSource`) to add a manual, on-demand "refresh live" button later.
- **Weekly/rolling *reset* reconstruction for Claude.** Claude's local files do not expose a weekly reset clock; we show weekly *totals* only. (Codex *does* expose it — see §4.)
- **Scheduled background scans / notification firing.** That is Plan 10's remaining open fork and is tracked separately; this spec only *reads* the latest scan's critical count for the badge/popover.
- **`Accessory` (dock-hidden) activation policy.** Ward stays a normal windowed app (Dock icon retained) for v1.

---

## 2. Prior art & licensing

We port algorithm and pricing logic (not literal UI) from these projects. All are permissive; **no GPL/copyleft** in the set. Attribution is added to the existing `NOTICE` file, exactly as done for the CCO reference.

| Project | License | What we take |
|---|---|---|
| `ryoppippi/ccusage` | MIT © 2025 ryoppippi | Canonical 5-hour-block + dedup + pricing algorithm (golden reference) |
| `hydai/ccstat` | MIT | Faithful Rust port — primary implementation reference |
| `daybigo/ClaudeBar` | MIT | Rust + Tauri tray + session/weekly limit shape |
| `DaveDev42/ccusage-in-rust` | BSD-3 | "Bit-exact" parity — golden-test fixture shapes |
| BerriAI/LiteLLM `model_prices_and_context_window.json` | MIT | Model→price table (embedded at build) |

Reference-only (UX study, **not** copied — reimplemented in SolidJS): `Iamshankhadeep/ccseva` (MIT), `htahaozlu/context-bar` (Apache-2.0), `steipete/CodexBar` (MIT).

**Risk note:** the Approach-B "live API" path used by some of these tools depends on undocumented Anthropic headers; we deliberately do not use it (see Non-goals).

---

## 3. Architecture overview

```
src-tauri/src/
  usage/                     ← NEW (Plan 14): local usage engine
    mod.rs                   public API: usage_snapshot(harness) -> UsageSnapshot; shared models
    blocks.rs                5-hour-block algorithm (ports ccusage identify_session_blocks)
    pricing.rs               LiteLLM price table (embedded via build.rs), model→price lookup
    claude.rs                ~/.claude/projects/**/*.jsonl → blocks + weekly totals
    codex.rs                 ~/.codex/sessions/**/rollout-*.jsonl → token_count deltas + rate_limits
  native/
    tray.rs                  (edit) left-click toggles popover; tooltip from scan; template icon
    popover.rs               ← NEW (Plan 15): create/position/toggle the popover webview window
    autostart.rs             ← NEW (Plan 13): first-run enable gate + status/set helpers
    notify.rs / watch.rs     (unchanged)
  commands.rs                (edit) usage_snapshot, autostart_status, autostart_set
  lib.rs                     (edit) on_window_event close-to-tray; register plugins/commands
  build.rs                   (edit) fetch+embed pinned LiteLLM pricing JSON

src/
  index.tsx                  (edit) render <Popover> when window label === 'popover'
  entries/Popover.tsx        ← NEW (Plan 15): glance popover view
  api.ts                     (edit) usageSnapshot / autostartStatus / autostartSet + types
  styles/popover.css         ← NEW: popover styling (tokens/classes, no inline)
  mock/                      (edit) usage_snapshot + autostart fixtures for dev:mock

src-tauri/
  icons/                     (regenerated) from approved concept A (Plan 13)
  Cargo.toml                 (edit) tauri-plugin-autostart, tauri-plugin-positioner[tray-icon]
capabilities/default.json    (edit) autostart + positioner + window show/hide/set-focus perms
NOTICE                       (edit) attribution for ported projects
package.json                 (edit) @tauri-apps/plugin-autostart, @tauri-apps/plugin-positioner
```

---

## 4. Data sources (local-only)

### 4.1 Claude Code — reconstructed

- **Paths:** for each base in `[$XDG_CONFIG_HOME/claude, ~/.claude]` (and any `CLAUDE_CONFIG_DIR`, comma-split), walk `{base}/projects/**/*.jsonl` (recursive dir walk filtering `extension == "jsonl"`, then sort by path string — **not** the `glob` crate). Reuse `sessions::parse` machinery where possible.
- **Per-line schema** (outer entry is `camelCase`; the token object is `snake_case` on the wire — this asymmetry is a known gotcha):
  - entry: `timestamp` (ISO), `message`, `costUSD?`, `requestId?`, `sessionId?`, `isSidechain?`, `isApiErrorMessage?`
  - `message`: `usage`, `model?`, `id?`
  - `usage`: `input_tokens`, `output_tokens`, `cache_creation_input_tokens` (default 0), `cache_read_input_tokens` (default 0)
- **Dedup key:** `messageId:requestId`; if `requestId` absent, dedup on `messageId` alone; if `messageId` absent, count the entry but never dedup it. On collision keep the "better" entry (non-sidechain → higher total tokens).
- **Reset:** reconstructed from the 5-hour-block algorithm (§5). `percent` is `None` unless the user configures a plan limit (§6.4). Weekly = calendar-week token/cost totals (no reset clock).
- **Optional passive value:** if a line carries `usageLimitResetTime` (Claude writes it only when a limit was hit), surface it as an override for `block.resetsAt`. Never derived.

### 4.2 Codex CLI — authoritative

- **Paths:** `$CODEX_HOME` (default `~/.codex`, comma-split allowed) → `sessions/**/rollout-*.jsonl` (+ `archived_sessions/`).
- **Events:** lines where `payload.type == "token_count"`. Two payloads matter:
  1. **Cumulative token totals:** `input_tokens`, `cached_input_tokens`, `output_tokens`, `reasoning_output_tokens`, `total_tokens`. Per-turn spend = **delta of cumulative totals** between consecutive events.
  2. **`rate_limits`:** `primary` (rolling **5-hour** window) and `secondary` (**weekly** cap), each with `used_percent` and a reset timestamp; plus `plan_type`. This gives **real percent + real reset** with no network.
- **Model:** from the separate `turn_context` metadata; sessions before commit `0269096` (2025-09-06) lack usage and are skipped.
- **Bug guard:** on rate-limit-only updates Codex re-emits `token_count` with a stale non-zero `last_token_usage`. **Never** treat `last_token_usage` as the per-event increment — always diff cumulative totals.
- **Exact `rate_limits` field nesting is pinned from a real fixture** captured in Plan 14 Task 1 (see §9), because the payload shape has drifted across Codex releases.

### 4.3 Why local-only

- Codex already publishes the real numbers locally; Claude's 5h reset is deterministic. Token counts are exact from the files.
- The only "more accurate" alternative is scraping an undocumented Anthropic endpoint with the user's OAuth token — which contradicts Ward's no-auto-network rule and is flagged as a security smell by the community. `UsageSource` marks each snapshot (`Local` vs `RateLimits`) so a future manual refresh can be added without reshaping the model.

---

## 5. The 5-hour-block algorithm (Claude)

Ported from `ccusage` `identify_session_blocks` (`ccstat` mirrors it). All timestamps epoch-millis UTC; window = `SESSION_MS = 5 * 3_600_000 = 18_000_000`.

1. Sort entries ascending by timestamp; capture `now` once.
2. **Block start** = first entry's timestamp **floored to the top of the hour (UTC)**: `floor_to_hour(t) = (t / 3_600_000) * 3_600_000`.
3. Open a **new block** when the next entry is `> SESSION_MS` past the block **start** *or* `> SESSION_MS` past the **previous entry** (strict `>`, saturating subtraction).
4. **Reset / end time** = `start + SESSION_MS`. This is the countdown shown to the user (distinct from `actual_end` = last entry's timestamp).
5. **Active block** ⇔ has entries AND `now − last_entry < SESSION_MS` AND `now < end`. `resetsInSecs = (end − now)`.
6. **Gap blocks** (idle placeholders, `isGap: true`, zero tokens, never active) are inserted only when `since_last > SESSION_MS`; not surfaced in the popover but preserved so weekly aggregation and tests match ccusage.

The popover's Claude "current block" is the single **active** block (or the most recent block if none is active — shown as expired with `resetsInSecs <= 0`).

Golden fixtures assert: floor-to-hour start, `end = start + 5h`, active/expired classification, gap insertion, and dedup — cross-checked against ccusage-in-rust's documented parity behavior.

---

## 6. Data models & command API

### 6.1 Rust models (`usage/mod.rs`)

All derive `Debug, Clone, Serialize, Deserialize, PartialEq` with `#[serde(rename_all = "camelCase")]`, per project convention. Named to avoid collision with the existing `sessions::Usage`.

```rust
pub struct UsageSnapshot {
    pub harness: String,          // "claude" | "codex"
    pub block: UsageWindow,       // current 5-hour window
    pub week: UsageWindow,        // weekly cap (Codex) / weekly totals (Claude)
    pub source: UsageSource,      // Local | RateLimits
    pub available: bool,          // false when the harness dir is absent/empty
    pub generated_at: String,     // ISO8601
}

pub struct UsageWindow {
    pub tokens: TokenTotals,
    pub cost_usd: f64,
    pub percent: Option<f64>,     // 0.0..=1.0; Some for Codex, or Claude when plan configured
    pub resets_at: Option<String>,// ISO8601 (block end for Claude; rate_limits reset for Codex)
    pub resets_in_secs: Option<i64>,
    pub is_active: bool,
    pub started_at: Option<String>,
    pub plan_type: Option<String>,// Codex rate_limits.plan_type; None for Claude
}

pub struct TokenTotals { pub input: u64, pub output: u64, pub cache_creation: u64, pub cache_read: u64, pub total: u64 }

pub enum UsageSource { Local, RateLimits }   // serde: rename_all = "camelCase"
```

### 6.2 Commands (registered in `generate_handler!`)

- `usage_snapshot(harness: String) -> Result<UsageSnapshot, WardError>`
- `autostart_status() -> Result<bool, WardError>`
- `autostart_set(enabled: bool) -> Result<(), WardError>`

JS camelCase args map to Rust snake_case automatically (`harness` is a single word — unaffected).

### 6.3 Frontend types & wrappers (`api.ts`)

Mirror the Rust models as TS interfaces (`UsageSnapshot`, `UsageWindow`, `TokenTotals`, `UsageSource`), plus wrappers:
`usageSnapshot(harness) -> Promise<UsageSnapshot>`, `autostartStatus() -> Promise<boolean>`, `autostartSet(enabled) -> Promise<void>` (all via `invokeOrThrow`).

### 6.4 Optional Claude plan limit

Claude `percent` stays `None` by default (we show tokens/cost + countdown honestly). If the user sets a limit in `~/.ward/settings.json` (`{"claudeBlockTokenLimit": <u64>}`), `block.percent = block.tokens.total / limit`. No plan tier is hardcoded — the number is the user's own, avoiding shipping disputed constants. (A tier picker is a possible later addition; not in v1.)

---

## 7. Native shell behavior

### 7.1 Close-to-tray (`lib.rs`)

```rust
.on_window_event(|window, event| {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if !QUITTING.load(Ordering::SeqCst) {
            let _ = window.hide();       // hidden, not destroyed → agent keeps running
            api.prevent_close();
        }
    }
})
```

`QUITTING: AtomicBool` is set to `true` by (a) the tray **Quit** menu item (then `app.exit(0)`) and (b) macOS **⌘Q** (`RunEvent::ExitRequested`), so those genuinely terminate while the red button / ⌘W only hide. The main window is created **visible** (unchanged); reopen it via the tray **Open** menu item or the popover's **Open** button (`show` + `unminimize` + `set_focus`, reusing the existing `tray_action` `open` handler). Note: **tray left-click is repurposed to toggle the popover** (§7.3), so it no longer reopens the main window — the right-click **Open** menu item is the reopen path.

### 7.2 Launch at login (`native/autostart.rs`)

- Plugin: `tauri-plugin-autostart` with `MacosLauncher::LaunchAgent` (writes a per-user LaunchAgent plist — the modern mechanism, distinct from Plan 08's backup plist so they don't collide).
- **First-run default-on, without fighting the user:** on setup, if `~/.ward/first-run` sentinel is absent → if `!autolaunch().is_enabled()` then `enable()`; then create the sentinel. A later user "disable" persists because we never re-enable after the sentinel exists.
- Helpers wrap `ManagerExt`: `autostart_status()` → `is_enabled()`, `autostart_set(true|false)` → `enable()/disable()`. Errors map to `WardError` but are non-fatal.

### 7.3 Popover window (`native/popover.rs` + `tray.rs`)

- A dedicated webview window, label **`popover`**, created hidden at startup: `decorations(false)`, `always_on_top(true)`, `skip_taskbar(true)`, `resizable(false)`, ~`320×420`, loading the app URL with a `popover` marker (window label is enough; no separate HTML entry).
- `tauri-plugin-positioner` (Cargo feature **`tray-icon`** — required for the `Tray*` variants). The tray's `on_tray_icon_event` forwards `tauri_plugin_positioner::on_tray_event(...)` so the plugin caches the icon rect.
- **Left-click** the tray icon → toggle: if visible, `hide()`; else `move_window(Position::TrayCenter)` + `show()` + `set_focus()`. **Right-click** keeps the existing native menu (Open / Scan now / Quit). The popover hides on blur (`WindowEvent::Focused(false)` → `hide()`), like a standard macOS menu-bar popover.

### 7.4 Tray icon & badge

- Replace the mis-tinted default-window-icon tray glyph with a dedicated **monochrome template** PNG (`icons/tray-template.png`) + `icon_as_template(true)` so it adapts to light/dark menu bars.
- Tooltip via existing `format_tooltip(critical, last_scan_at)`; drive the already-present-but-orphaned `update_badge()` from the latest scan's critical count so the Dock badge reflects findings.

---

## 8. Frontend — glance popover

- **Routing (`index.tsx`):** `import { getCurrentWindow } from '@tauri-apps/api/window'`; if `getCurrentWindow().label === 'popover'` render `<Popover>`, else `<App>`. For `dev:mock` browser testing (no native window), also honor `?view=popover` so the popover renders at `http://localhost:1430/?view=popover`.
- **`entries/Popover.tsx`:** on mount, `Promise.all([api.usageSnapshot('claude'), api.usageSnapshot('codex')])`; also read the latest scan's critical count (reuse the Security resource / `scan-now` event) and `api.autostartStatus()`. Renders, per harness: a labeled usage **gauge** (mint bar; `percent` when present, else a tokens/cost readout), a **reset countdown** (`resetsInSecs` ticking down, formatted `2h41m` / absolute time), and `plan_type` if present. Footer: **critical-findings** count, **Scan now** (emit `scan-now`), **Open** (show main window), and a **Launch at login** toggle (`api.autostartSet`). Subscribes to `config-changed` / `scan-now` to refresh, and cleans up on unmount (matching `Security.tsx`'s listener pattern).
- **Styling:** new `src/styles/popover.css`, imported by the component; class-based using existing tokens (`--bg`, `--surface`, `--accent` mint, `--crit`, `--warn`, `--ok`, radii, shadows). No inline styles (so `:hover`/transitions/`::selection` work). Gauge fill color ramps ok→warn→crit by percent when a percent exists.
- **Unavailable harness:** `available: false` renders a muted "No ~/.codex usage found" row rather than an error.

---

## 9. App icon

- Source: approved **concept A** (glowing mint shield on deep-navy circuit grid). Post-process the generated 1024² PNG so everything **outside the squircle is transparent** (macOS does not auto-mask third-party `.icns`), producing `src-tauri/icons/icon-source.png`.
- Run `npm run tauri icon src-tauri/icons/icon-source.png` (a.k.a. `tauri icon`) to regenerate `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns`, `icon.ico`, and Windows/Store variants referenced by `tauri.conf.json`.
- Derive a separate **monochrome template** (`icons/tray-template.png`, white shield on transparent) for the menu bar.
- Commit all regenerated binaries.

---

## 10. Error handling

- Missing/empty `~/.claude` or `~/.codex` → `UsageSnapshot { available: false, .. }`, **not** an error.
- Malformed JSONL lines are skipped (parse-tolerant), matching ccusage.
- Unknown model in pricing lookup → cost `0.0` (never throws).
- Pricing embed absent at build → build fails loudly (no silent zero-price binary).
- Autostart enable/disable failures → `WardError`, surfaced in the popover toggle, non-fatal to the app.
- Popover window creation failure → logged; tray Open/main window still work (graceful degrade).

All errors reuse the existing `WardError` (`thiserror` + manual `Serialize` with `#[serde(tag="kind", content="message")]`).

## 11. Testing strategy

TDD, every `cargo test` / `npm test` green before a task is done (project quality bar).

**Rust (Plan 14):**
- `blocks.rs`: floor-to-hour start; `end = start + 5h`; active vs expired; gap insertion; boundary at exactly 5h (strict `>`). Golden fixtures mirror ccusage-in-rust.
- Dedup: `messageId:requestId`, missing-requestId, missing-messageId (counted, not deduped), sidechain collision resolution.
- `pricing.rs`: exact + fuzzy model match; unknown → 0; cache-token pricing defaults.
- `claude.rs` / `codex.rs`: aggregate from committed fixture files under `src-tauri/tests/fixtures/usage/`; Codex cumulative-delta correctness + the `last_token_usage` re-emit-bug guard + `rate_limits` percent/reset parse.

**Rust (Plan 13):** autostart first-run gate logic (sentinel present/absent → enable decision) with a temp `WARD_HOME`; `QUITTING` flag gating of close-to-tray (unit-level where the event path is testable).

**JS (Plan 15):** `<Popover>` renders gauges/countdowns/critical-count/toggle from a mock `UsageSnapshot`; window-label routing selects `<Popover>` vs `<App>`; autostart toggle calls the wrapper. New `usage_snapshot` + `autostart_status` fixtures in `src/mock/`.

**Manual (macOS):** `tauri-driver` can't run on macOS, so verify the popover and toggles via `npm run dev:mock` + Chrome DevTools (`?view=popover`); verify real close-to-tray / launch-at-login / tray popover in `npm run tauri dev` as a hands-on step.

---

## 12. Plan breakdown

### Plan 13 — Native shell UX
**Goal:** close-to-tray, launch-at-login, app icon, tray tooltip/badge.
**Files:** `lib.rs` (on_window_event + QUITTING + ⌘Q), `native/autostart.rs`, `native/tray.rs` (template icon, badge/tooltip wiring, left-click reserved for popover in Plan 15), `commands.rs` (`autostart_status/set`), `Cargo.toml` + `package.json` (autostart plugin), `capabilities/default.json`, `icons/*` regenerated, `tauri.conf.json` if icon paths change, `NOTICE`.
**Tasks:** (1) autostart plugin + first-run gate + status/set commands + tests; (2) close-to-tray window handler + real-quit paths; (3) icon pipeline (transparent source → `tauri icon` → template glyph, commit); (4) tray tooltip + Dock badge from scan.
**Tests:** autostart gate unit tests; JS wrappers. **Gotchas:** don't re-enable autostart after user disables (sentinel); Quit vs red-button distinction; macOS icon needs transparent corners.

### Plan 14 — Usage engine (Rust)
**Goal:** `usage/` module + `usage_snapshot` command, fully golden-tested, offline.
**Files:** `usage/{mod,blocks,pricing,claude,codex}.rs`, `build.rs` (embed pinned LiteLLM JSON), `commands.rs` (`usage_snapshot`), `lib.rs` (register), `tests/fixtures/usage/*`, `NOTICE`.
**Tasks:** (1) capture a real `~/.codex` `token_count` fixture to pin `rate_limits` nesting; (2) `blocks.rs` + golden tests; (3) `pricing.rs` + build embed + offline test; (4) `claude.rs` aggregation + dedup tests; (5) `codex.rs` delta + rate_limits tests; (6) `usage_snapshot` command + models.
**Gotchas:** camelCase-outer / snake_case-token asymmetry; Codex `last_token_usage` bug; recursive walk not glob; unknown model → 0.

### Plan 15 — Glance popover (UI)
**Goal:** the popover window + `<Popover>` view wired to `usage_snapshot` + findings + autostart.
**Files:** `native/popover.rs`, `native/tray.rs` (left-click toggle + positioner forwarding), `lib.rs` (positioner plugin, popover window, blur-hide), `Cargo.toml`+`package.json` (positioner), `capabilities/default.json`, `src/index.tsx` (label routing), `src/entries/Popover.tsx`, `src/styles/popover.css`, `src/mock/*` (usage fixtures).
**Tasks:** (1) popover window + positioner + tray left-click toggle + blur-hide; (2) index label/`?view` routing; (3) `<Popover>` component + styling + live countdown; (4) wire usage + critical count + autostart toggle; (5) mock fixtures + JS tests.
**Gotchas:** positioner needs the `tray-icon` feature; forward `on_tray_event`; popover reuses the bundle (no second HTML); dev:mock has no native window (`?view=popover`).

---

## 13. Open decisions (resolved)

- **Usage data source →** local-only (Codex authoritative, Claude reconstructed). Live API deferred behind `UsageSource`.
- **Menu-bar depth →** glance popover (webview), matching the locked spec's feature 16.
- **Launch-at-login default →** on, but gated by a first-run sentinel + visible toggle (not re-forced).
- **Activation policy →** stay `Regular` (keep Dock icon) for v1.
- **Icon →** concept A (shield + circuit grid).
- **Scope →** all three, sequenced as Plans 13 → 14 → 15.
