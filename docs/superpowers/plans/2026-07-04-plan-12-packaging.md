# Ward Plan 12 — Packaging: .dmg / signing / E2E / polish (High-Level)

> High-level plan. Use **superpowers:subagent-driven-development**; several steps are manual/credential-gated. Commit per task. CCO reference (read-only): `/Users/balakumar/personal/claude-code-organizer`.

**Goal:** Ship Ward — signed, notarized `.dmg`, end-to-end tests, and final polish.

**Builds on:** everything (Plans 01–11).

**Files / areas:**
- Modify `src-tauri/tauri.conf.json`: `bundle` → macOS `dmg`, category `public.app-category.developer-tools`, full icon set; **universal** target (`aarch64-apple-darwin` + `x86_64-apple-darwin`).
- Signing + notarization: Developer ID cert (**user-provided**), env vars `APPLE_CERTIFICATE`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` — document, never hardcode.
- Optional: Tauri **updater** plugin.
- Create `tests/e2e/` with **`tauri-driver` + WebDriver**: core flows (scan → find → act → undo, move, budget, sessions, backup).
- Polish: light/dark parity, keyboard navigation, empty states, error toasts (surface `WardError.kind`).

**Task checklist:**
- [ ] Bundle config → `npm run tauri build` produces a `.dmg`.
- [ ] Universal (Apple Silicon + Intel) build.
- [ ] Signing + notarization (with user-provided certs).
- [ ] E2E harness + core-flow specs.
- [ ] Polish pass (dark/light, keyboard, empty/error states).

**CCO parity refs:** `tests/e2e/dashboard.spec.mjs` (behavioral contract to mirror as WebDriver specs).

**Tests:** `tauri build` yields a launchable signed `.dmg`; E2E core flows pass; app runs on a clean macOS user with no dev toolchain.

**Gotchas:** signing/notarization need Apple certs + secrets (**user provides — do not hardcode**); notarization has latency (minutes); universal build doubles compile time; verify Gatekeeper acceptance on a machine that never ran the dev build.
