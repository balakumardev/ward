# Ward E2E Tests (Plan 12)

End-to-end specs that drive a running Ward.app via
[`tauri-driver`](https://github.com/tauri-apps/tauri/tree/dev/crates/tauri-driver)
+ WebDriverIO. The webview is WebKit-based, so no separate browser
binary is required.

**Specs are NOT wired into CI.** They are run by hand against a real
desktop session. CI on this repo runs `cargo test`, `vitest run`,
`tsc --noEmit`, and `vite build` — not the E2E suite.

## Layout

```
tests/e2e/
├── package.json       # local devDeps + npm scripts
├── tsconfig.json      # standalone tsc config (does not affect /tsconfig.json)
├── wdio.conf.ts       # WebDriverIO config (capabilities, specs list)
├── README.md          # this file
├── smoke.spec.ts      # sidebar 5 modes + 12 category labels (dashboard smoke)
├── scan.spec.ts       # scan → find via Organizer search input
├── move.spec.ts       # move modal + delete-with-undo toast
├── budget.spec.ts     # Context Budget mode (Plan 06)
├── sessions.spec.ts   # Sessions mode (Plan 07)
├── backup.spec.ts     # Backups mode (Plan 08)
└── security.spec.ts   # Security mode + Scan now button (Plan 05)
```

## Prerequisites

1. **macOS 11+** (Ward targets `minimumSystemVersion = "11.0"`).
2. **Tauri CLI** — already installed for the parent repo (`@tauri-apps/cli`).
3. **tauri-driver** — install once:
   ```bash
   cargo install tauri-driver --locked
   ```
4. **A built `Ward.app`** for the tauri-driver to spawn:
   ```bash
   ./src-tauri/dist/sign.sh        # or `npm run tauri build` for unsigned
   ```
5. **An Apple account with `~/.claude/` or `~/.codex/` populated** —
   the tests rely on the Organizer scan finding real config.

## Running

```bash
cd tests/e2e
npm install                                  # one-time
WARD_APP_PATH=/abs/path/to/Ward.app npm run test:e2e       # full suite
WARD_APP_PATH=/abs/path/to/Ward.app npm run test:e2e:smoke # just the smoke test
```

`tauri-driver` listens on `127.0.0.1:4444` by default — change
`wdio.conf.ts > port` if your machine already uses that port.

## What the smoke test actually checks

`smoke.spec.ts` is the canonical "did the app render correctly" test,
modeled on CCO's `dashboard.spec.mjs`. It asserts:

1. **Sidebar with 5 modes** — Organizer, Security, Context Budget,
   Sessions, Backups. The count is enforced by
   `src/components/Sidebar.tsx > MODES`.
2. **Organizer renders 12 category labels** — Skills, Memories, MCP,
   Commands, Agents, Plans, Rules, Config, Hooks, Plugins, Sessions,
   Settings. The list comes from
   `src-tauri/src/harness/framework.rs > category_label()`.
3. **No console errors** — same collector as CCO's spec.

The other specs are loose smoke probes that degrade gracefully when
the user's HOME doesn't have the matching config. They are scaffolds;
new assertions can be added without reshaping the harness.

## Selectors

The spec files use `data-testid` selectors that **must** be present in
the production renderer. Today the app exposes:

- `[data-testid="sidebar"]`
- `[data-testid="harness-select"]`
- `[data-testid="scan-loading"]`
- `[data-testid="mcp-policy-button"]`
- `[data-testid="organizer-item"]`
- `[data-testid="move-modal"]`, `[data-testid="move-cancel"]`
- `[data-testid="delete-modal"]`, `[data-testid="delete-confirm"]`
- `[data-testid="toast"]`, `[data-testid="toast-undo"]`
- `[data-testid="budget-panel"]`, `[data-testid="budget-unsupported"]`
- `[data-testid="sessions-panel"]`, `[data-testid="sessions-unsupported"]`
- `[data-testid="backups-panel"]`, `[data-testid="backups-unsupported"]`
- `[data-testid="security-panel"]`, `[data-testid="security-scan-now"]`

If you add a new UI node that E2E specs need to touch, add a matching
`data-testid` to the JSX.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `ECONNREFUSED 127.0.0.1:4444` | tauri-driver isn't running | `tauri-driver --port 4444` in another shell |
| `Cannot find module '@wdio/globals'` | `npm install` not run in `tests/e2e/` | `cd tests/e2e && npm install` |
| TypeScript compile errors when running specs | Specs drift from current UI | `npm run typecheck` (uses local `tsconfig.json`) |
| Specs hang on first action | App splash screen | Increase `mochaOpts.timeout` in `wdio.conf.ts` |
