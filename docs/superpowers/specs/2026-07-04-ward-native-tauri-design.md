# Ward — Native macOS Config Organizer — Design Spec

**Date:** 2026-07-04
**Status:** Approved (design) → ready for implementation planning
**Working title:** Ward (`Ward.app`, repo `~/personal/ward`)

---

## 1. Overview

Ward is a **native macOS desktop app (Tauri 2.0)** that organizes the configuration of AI coding harnesses — **Claude Code and Codex CLI** — from one window. It is a **config organizer first**, with an integrated **MCP security scanner**, **context-window budget** view, **session** tools, and a **git-backed backup center**. A background **menu-bar agent** runs scheduled security scans and raises native notifications.

Ward is a **from-scratch reimplementation** of the feature set pioneered by the open-source web app **Cross-Code Organizer (CCO)** (`@mcpware/cross-code-organizer`, MIT). It reuses **none** of CCO's source: the backend is rewritten in **Rust**, and the frontend is a **fresh native-first redesign**. CCO is credited as design/feature inspiration (see §12).

**One-line identity:** *A Mac-native command center for everything your AI coding tools silently load — see it, clean it up, and scan it for poisoned MCP servers.*

### Why native (the reason to build this vs. the existing web app)
- A real double-click `.dmg` — no Node, no terminal, no browser tab.
- A **menu-bar presence** with background scheduled scans and **native notifications** — impossible in a web app.
- Native file dialogs, Finder integration, filesystem watching for live refresh.
- The smallest, signable, self-contained native binary (Rust backend, no bundled runtime).

### What it is NOT
- Not a Claude Code **session runner** (that lane is owned by opcode/Claudia @ ~22k★). Ward does not run agent sessions; it *organizes and secures their config*.
- Not "just a security app" — the scanner is one of five sidebar modes.

---

## 2. Competitive positioning (why this shape)

Prior-art research (2026-07-04, ~60 tools surveyed) found the space fragmented:
- **Cross-harness config** is a *crowded* narrative led by `cc-switch` (~113k★) — so Ward does **not** pitch on "manages Claude + Codex in one place" alone.
- **Genuinely unoccupied pillars** — Ward's wedge — are: **(a) MCP/skill security scanning integrated into a manager**, **(b) effective-config resolution** (merged global→project→user), **(c) context-*window* budget composition** (not $ cost), and **(d) Codex `config.toml` depth**. Almost no desktop tool combines even three of these.
- Note: the most *native-feeling* rivals are Swift apps; Tauri's win here is **code-clarity + one language for logic + cross-platform later**, not "more native than Swift." The UI is a web view regardless — polish must be earned in the redesign.

---

## 3. Goals & non-goals

**Goals**
1. **Full feature parity** with CCO's shipping product (§4), reimplemented natively.
2. Native macOS UX: menu-bar agent, notifications, `.dmg`, light/dark, file-watching.
3. Preserve CCO's differentiators exactly: Show Effective, click-a-finding→act, per-item context budget, undo, bulk ops, cross-harness capability-gating.
4. Zero telemetry; all processing local (parity with CCO's privacy stance).

**Non-goals (v1)**
- Windows/Linux packaging (architecture stays cross-platform-friendly, but only macOS is shipped/tested first).
- Porting CCO's `research/` activation-probe ML lane (it is not wired into CCO's shipping product and is out of scope).
- Running or proxying live agent sessions.

---

## 4. Feature parity inventory (the "everything" list)

Every CCO shipping feature, reimplemented:

| # | Feature | Notes |
|---|---|---|
| 1 | **Organizer** browse across categories | Claude: skill, memory, mcp, command, agent, plan, rule, config, hook, plugin, session, setting (12). Codex: config, memory, skill, mcp, profile, rule, plugin, session, history, shell, runtime (11). |
| 2 | Scope-grouped items (global / project) | With movable / deletable / locked flags. |
| 3 | **Move** item between scopes | With valid-destination resolution. |
| 4 | **Delete** item + **Undo** (restore) | Live undo via operations layer (file + MCP-entry restore). |
| 5 | **Frontmatter / content editor** | Edit skills/agents/memories markdown; save. |
| 6 | **Show Effective** resolution | Per-category shadow/conflict/ancestor rules (Claude only). |
| 7 | **MCP enable/disable per project** | Disabled-server list. |
| 8 | **MCP policy** (allow/deny) | User-settings allowlist/denylist. |
| 9 | **MCP security scanner** (4-layer) | Deobfuscation → pattern rules → hash baseline → optional LLM judge. §6.3 |
| 10 | **Context budget** breakdown | System overhead + per-item token composition; CLAUDE.md `@import` expansion. |
| 11 | **Sessions** viewer | Parse JSONL → conversation; per-model cost; **distill** (~90% cut) + **image trim**. |
| 12 | **Backup center** | Export → git commit → push; **scheduled** backups (launchd). |
| 13 | **Bulk operations** | Multi-select move/delete. |
| 14 | **Ward-as-MCP-server** | Expose Ward's scan/move/delete/audit as MCP tools (stdio) for AI clients. |
| 15 | **Cross-harness** capability-gating | UI hides features a harness reports unsupported (e.g. Codex has no effective/budget/mcpControls). |

**Native-only additions**
| 16 | **Menu-bar agent** | Glance popover: top findings, last-scan, schedule, Scan-now, Open. |
| 17 | **Background scheduled scans** | launchd; fires **native notifications** on new critical findings. |
| 18 | **Filesystem watching** | Live refresh when `~/.claude` / `~/.codex` / project config changes. |

---

## 5. Locked design decisions (from visual brainstorming)

| Decision | Choice |
|---|---|
| Backend | **Full Rust rewrite** — all logic as Tauri `invoke` commands (no bundled Node). |
| Frontend | **Fresh native-first redesign** (macOS HIG); reuse only feature set + data shapes, no CCO UI code. |
| **App shell** | **C — Hybrid:** top-level *modes* in the sidebar (Organizer · Security · Context Budget · Sessions · Backups). Organizer is a 3-column browse (categories → items → detail); the heavy views take the full width. |
| **Visual style** | **Security Console** — dark, status-color-forward (critical/warn/ok), severity-bordered cards; scanner as visual hero; carries the general organizer fine. |
| **Security flow** | **In-place master–detail** — findings list + click-to-open the rule, highlighted poisoned snippet, server config, and all actions, without leaving Security mode. |
| **Menu-bar** | **Glance + alert** — badge on findings, popover glance, background scan + notifications. (Control-center popover is a deferred phase-2.) |
| Harness switcher | Top-right of the sidebar; Claude Code ⇄ Codex CLI. |

---

## 6. Architecture

### 6.1 Top-level shape
```
┌──────────────────────────────────────────────┐
│  Ward.app (Tauri 2.0)                          │
│                                                │
│  ┌─────────────┐   invoke()    ┌────────────┐  │
│  │  WebView UI │ ◄───────────► │  Rust core │  │
│  │  (Solid+TS) │   events      │  (commands)│  │
│  └─────────────┘               └─────┬──────┘  │
│                                      │          │
│  Native shell (Rust):  tray · notifications ·  │
│  launchd scheduler · fs-watch · window mgmt    │
└──────────────────────────────────────────────┘
        │ reads/writes            │ shells out
        ▼                          ▼
  ~/.claude, ~/.claude.json,   git · launchctl ·
  ~/.codex, <repo>/.claude…    claude CLI · MCP subprocesses
```

The UI never touches the filesystem. It calls Rust **`invoke` commands** (replacing CCO's ~30 HTTP endpoints) and subscribes to **events** (scan progress, fs-change, scheduled-scan results).

### 6.2 Rust crate/module layout (mirrors CCO's clean adapter seam)
```
src-tauri/src/
  main.rs                 // Tauri builder, command registry, tray, plugins
  commands/               // thin invoke handlers → core (1:1 with CCO endpoints)
    scan.rs  move.rs  delete.rs  restore.rs  file.rs  settings.rs
    context_budget.rs  security.rs  mcp_controls.rs  backup.rs
    sessions.rs  export.rs  destinations.rs
  harness/
    mod.rs                // Harness trait + registry (auto-register adapters)
    model.rs              // ScanResult, HarnessItem, Capabilities, Scope, Category
    framework.rs          // generic scan engine (discover→scan categories→normalize)
    fs_utils.rs           // path decoding, safe reads, $HOME confinement
    adapters/
      claude.rs           // 12 categories, effective model, mcp policy, budget
      codex.rs            // 11 categories, TOML parsing
      claude_ops.rs       // move/delete/restore + valid destinations
      claude_budget.rs    // token composition, @import expansion
  security/
    mod.rs                // 4-layer pipeline orchestration
    deobfuscate.rs        // 8 techniques (base64, homoglyph, zero-width, …)
    rules.rs              // ~40+ regex rules (PI/TP/TS/SF/DE/CH/CE/CI/HK/EP)
    baseline.rs           // hash store/compare (~/.claude/.cco-security → ~/.ward/…)
    judge.rs              // optional `claude -p` LLM judge
  mcp/
    introspect.rs         // JSON-RPC over stdio + streamable HTTP; hash tool defs
    server.rs             // Ward-as-MCP-server (stdio) — scan/move/delete/audit tools
  sessions/
    parse.rs  cost.rs  distill.rs  trim.rs
  backup/
    git.rs                // init/remote/commit/push
    scheduler.rs          // launchd plist install/remove/status (macOS)
  tokenizer.rs            // tiktoken-rs; fallback bytes/4
  native/
    tray.rs  notify.rs  watch.rs   // menu-bar, notifications, fs-watch
```

### 6.3 Key subsystem notes
- **Harness trait** (extensibility core, parity with CCO's adapter contract): `id, display_name, short_name, icon, executable, categories[], scope_types[], capabilities{context_budget, mcp_controls, mcp_policy, mcp_security, sessions, effective, backup}`, plus `get_paths()`, `discover_scopes()`, a `scanners` map (one per category), and optional `operations` (move/delete/restore/destinations), `effective`, `context_budget`, `security`, `mcp_controls`, `mcp_policy`. Adding a harness = add one adapter module + register. The UI is **capability-driven**.
- **Security scanner** — pure-Rust reimplementation of the 4 layers: (1) deobfuscation (8 techniques), (2) ~40+ regex rules grouped `prompt_injection` / `tool_poisoning` / `tool_shadowing` / `sensitive_access` / `data_exfiltration` / `credential_harvest` / `code_execution` / `command_injection` / `suspicious_hook` / `exfil_params`, (3) tool-definition hash baseline (detects a server silently changing tools), (4) optional user-triggered `claude -p` judge. Tool definitions come from the MCP introspector.
- **MCP introspector** — spawn MCP servers over stdio (and streamable HTTP), speak JSON-RPC, list tools, hash definitions (~200 lines in CCO → Rust). Consider the official Rust MCP SDK (`rmcp`) vs. hand-rolled client (decide in planning; hand-rolled matches CCO's control).
- **Context budget** — reproduce CCO's model: system-overhead constants, ~3100 tokens/unique MCP server, CLAUDE.md `@import` expansion (depth ≤5), autocompact/warning buffers. Tokenizer via `tiktoken-rs`; degrade to bytes/4 when unavailable (parity with CCO's optional `ai-tokenizer`).
- **Backups** — `git` operations (shell out, like CCO, or `git2`); scheduler writes a **launchd** plist (`com.balakumar.ward.backup`) + `launchctl` (macOS-only branch; CCO's systemd branch dropped).
- **Native shell** — Tauri tray for the menu-bar glance popover; `tauri-plugin-notification` for native alerts; `notify` crate for fs-watch → emits a `config-changed` event the UI listens to for live refresh; a launchd job (or in-process timer while running) drives scheduled scans.

### 6.4 Frontend stack (recommended; confirm in planning)
- **SolidJS + TypeScript + Vite** — small, fine-grained-reactive, ideal for dense item lists and live-updating meters; excellent Tauri story. *(Svelte 5 is an equally valid alternative; React is heavier than needed.)*
- **Design system:** CSS custom properties encoding the **Security Console** tokens (bg/surface/border, status colors critical/warn/ok, accent, mono/UI type). Custom native-feeling components (sidebar source-list, 3-column split, severity cards) — no heavy component library.
- **State:** per-mode stores; a thin `api.ts` wrapping `invoke()` calls (the single UI↔core seam).

### 6.5 Data flow (example: security scan)
1. User clicks **Scan now** → UI `invoke('security_scan', {harness})`.
2. Rust: introspect MCP servers → run layers 1–3 → return findings + baseline diff; stream progress via events.
3. UI renders findings (master list). Click a finding → detail (rule, deobfuscated snippet, server config, actions).
4. Action (e.g. **Disable for project**) → `invoke('mcp_set_disabled', …)` → fs write → re-scan affected scope → UI updates. All reversible via `restore`.
5. Background: launchd fires scan → new critical → `tauri-plugin-notification` → menu-bar badge updates.

---

## 7. Filesystem surface & safety
- **Reads/writes:** `~/.claude/` (skills, agents, commands, rules, plans, memory, projects/, plugins/cache, settings*.json, .mcp.json), `~/.claude.json` (user MCP), `~/.codex/` (config.toml, memories, skills, rules, sessions, history.jsonl, shell_snapshots), and discovered project dirs (`<repo>/.claude/*`, `.mcp.json`, `CLAUDE.md`, `AGENTS.md`, `.codex/config.toml`).
- **Ward's own state:** `~/.ward/` (security baselines, scan cache, prefs) — kept out of `~/.claude` to avoid colonizing another tool's dir.
- **Path safety:** confine all file commands to under `$HOME`; reject `../` traversal; canonicalize before access.
- **Privacy:** no telemetry; the only network calls are the optional update check and user-initiated git push. The `claude -p` judge runs locally.

---

## 8. Error handling
- Every `invoke` command returns `Result<T, WardError>`; `WardError` is a typed enum (`NotFound`, `PermissionDenied`, `PathEscaped`, `HarnessUnavailable`, `McpIntrospectFailed`, `GitFailed`, `SchedulerFailed`, `ParseError`) serialized to the UI with a user-facing message + machine code.
- **Degrade gracefully:** missing `claude` CLI → judge disabled, layers 1–3 still run; missing tokenizer → bytes/4 estimate flagged "estimated"; MCP server that won't introspect → shown as "unscannable," not a hard failure.
- **Mutations are reversible:** move/delete capture enough to restore; surface an Undo affordance after each.

---

## 9. Testing strategy
- **Rust unit tests** port CCO's unit suite intent (its strongest coverage): adapter regressions, path decoding correctness, effective-mode shadow/conflict rules, move-destination validation, security pattern hits/misses, Codex TOML parsing. This is the **primary safety net for the rewrite** — build a fixture corpus mirroring CCO's `tests/unit` + `tests/fixtures`.
- **Golden/parity tests:** run the same fixture config trees through Ward and assert the normalized `ScanResult` matches CCO's expected shapes (catch rewrite regressions).
- **Integration:** Tauri command tests over temp `$HOME` fixtures.
- **E2E (later):** WebDriver (`tauri-driver`) for the core flows (scan → find → act → undo, move, budget, sessions, backup).

---

## 10. Platform & distribution
- **macOS-first** (Apple Silicon + Intel universal). Cross-platform kept feasible but unshipped.
- `.dmg` via Tauri bundler; Developer ID signing + notarization for Gatekeeper (set up in a later phase; unsigned dev builds fine meanwhile).
- Auto-update via Tauri updater (later phase).

---

## 11. Risks & mitigations
| Risk | Mitigation |
|---|---|
| Rust rewrite drifts from CCO's subtle logic (40+ regexes, effective rules, path decoding) | Golden/parity fixture tests (§9) built *before/with* each subsystem; port CCO's test cases as the spec of record. |
| MCP introspection = spawning untrusted servers to read tool defs | Same trust model as CCO; run introspection with least privilege; never auto-run on untrusted configs without user action; document the caveat. |
| `claude -p` judge availability/format churn | Optional layer, feature-detected; never blocks layers 1–3. |
| launchd scheduler quirks | Isolate in `backup/scheduler.rs` + `native/`; status-check and surface failures as `SchedulerFailed`. |
| Tauri won't feel "more native" than Swift rivals | Invest the redesign in genuine macOS idioms (source-list sidebar, vibrancy, keyboard nav, HIG spacing); measure against SkillDeck/Claudoscope. |
| Scope creep (control-center popover, cross-platform) | Explicitly deferred to phase 2+. |

---

## 12. Attribution & license
- Original: **Cross-Code Organizer (CCO)**, `@mcpware/cross-code-organizer`, **MIT** (© 2026 mcpware).
- Ward reuses **no verbatim CCO code** (Rust backend + fresh UI = clean reimplementation; functionality/ideas are not copyrightable). MIT imposes no obligation on a clean reimplementation, but Ward ships a **`NOTICE`** crediting CCO as the feature/design inspiration as good-faith acknowledgment.
- If any file is ever lifted verbatim later, its MIT header + mcpware copyright must be retained.

---

## 13. High-level phasing (detailed plan follows in writing-plans)
0. Scaffold: Tauri 2 + Solid + Rust; design tokens; static C-shell.
1. Harness framework + Claude adapter **read-only** scan → Organizer browse + detail + Show Effective.
2. Mutations: move/delete/undo, frontmatter save, bulk; MCP enable/disable + policy.
3. Security mode: 4-layer scan + in-place master-detail + baseline + judge.
4. Context Budget mode + tokenizer.
5. Sessions: viewer, cost, distill, trim.
6. Backups: git + launchd scheduler.
7. Codex adapter (capability-gated parity).
8. Native shell: menu-bar glance+alert, background scan, notifications, fs-watch.
9. Ward-as-MCP-server mode.
10. Packaging: `.dmg`, signing/notarization, E2E, polish.

---

## 14. Open questions (resolve in planning)
1. Frontend framework — **SolidJS** (recommended) vs. Svelte 5? (Lock before phase 0.)
2. MCP client — official `rmcp` SDK vs. hand-rolled JSON-RPC (as CCO)?
3. Git — shell out to `git` (as CCO) vs. `git2` crate?
4. Scheduled scans — pure launchd job invoking a Ward CLI subcommand, vs. in-process timer while the app/menu-bar runs? (Affects whether scans run when the app is fully quit.)
