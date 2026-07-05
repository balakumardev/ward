# Plan 15 — Glance Popover (UI) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A menu-bar glance popover — a small webview anchored under the tray icon showing Claude & Codex usage (tokens/cost, percent, reset countdown) with a launch-at-login toggle and Open/Scan-now actions — plus a dock badge + tray tooltip reflecting the latest scan's critical findings.

**Architecture:** The backend usage/autostart commands already exist (Plan 14/13). This plan adds the TS wrappers (`api.ts`), the mock bridge cases (dev:mock preview), the `<Popover>` SolidJS component + `popover.css`, window-label routing in `index.tsx`, a native `popover` webview window toggled by the tray left-click via `tauri-plugin-positioner`, and a `native_update_status` command that Security calls after each scan to drive the badge + tooltip.

**Tech Stack:** SolidJS + TS + Vite (frontend), Rust/Tauri v2 (`tauri-plugin-positioner`), reusing the existing `usage_snapshot`/`autostart_*` commands.

## Global Constraints

- **Tauri v2 only.** `invoke` from `@tauri-apps/api/core`; new command registered in `lib.rs` `generate_handler!`. JS camelCase args → Rust snake_case automatic.
- **Class-based styling** via a new `src/styles/popover.css` imported by the component — use the existing tokens from `src/styles/tokens.css` (e.g. `--bg #0e1420`, `--surface #131c2b`, `--surface-3 #1a2536`, `--accent #8ff0a8`, `--crit #ff453a`, `--warn #ff9f0a`, `--ok #30d158`, `--text #dfe7f2`, `--text-dim`, `--border`, `--r-md 10px`, `--r-pill 999px`, `--sh-2`, `--glow`). NO inline styles for hover/transitions.
- **Frontend ↔ core ONLY via `invoke`** (the `api.ts` wrappers). The UI never touches the FS.
- **Models/types** mirror the Rust `usage::mod` structs exactly (camelCase): `UsageSnapshot { harness, block: UsageWindow, week: UsageWindow, source: 'local'|'rateLimits', available, generatedAt }`, `UsageWindow { tokens: TokenTotals, costUsd, percent?, resetsAt?, resetsInSecs?, isActive, startedAt?, planType? }`, `TokenTotals { input, output, cacheCreation, cacheRead, total }`. Optional fields are absent (not null) when unset — type them `?`.
- **Preserve `data-testid`s** on anything the tests/e2e depend on; add `data-testid` to new popover elements.
- **Errors:** `WardError` (Rust) unchanged; the new `native_update_status` returns `Result<(), WardError>`.
- **One commit per task**, conventional prefix (`feat(plan15):`). No stubs/TODOs.
- **Tests:** `npm test` (vitest) and `npx tsc --noEmit` must pass for frontend tasks; `cargo test --lib` + `cargo check` for backend tasks. macOS can't run `tauri-driver`; verify the popover via `npm run dev:mock` + `?view=popover` in Chrome DevTools (hands-on).

**Verified integration facts (current code):**
- `api.ts`: `isTauri()`, `invokeOrThrow<T>(cmd, args)`; the `export const api = { … }` object spans ~lines 260–317 (append wrappers before the closing `};`); type interfaces live ~lines 35–258 (add a `// ── Plan 14 — Usage engine ──` block).
- `mock/dispatch.ts`: a `switch (cmd)` with a `default: throw`; `mock/store.ts`: a `MockStore` returning `clone(fixture)`; `mock/fixtures.ts`: exported consts typed against `../api`. `mock/install.ts` hardcodes `metadata.currentWindow.label = 'main'`.
- `index.tsx`: `boot()` installs the mock when `import.meta.env.VITE_WARD_MOCK`, then `render(() => <App/>, root)`.
- `Security.tsx`: `createResource(() => api.securityScan('claude', props.items))`; critical count = `scan()?.severityCounts.critical`; last scan time = `scan()?.timestamp`; it already `listen('config-changed')` / `listen('scan-now')` (from `@tauri-apps/api/event`).
- `native/tray.rs` `setup()`: `show_menu_on_left_click(false)`; `on_tray_icon_event` currently emits a bare `tray_clicked` on left-click-up (free to repurpose). Imports `tauri::tray::{MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent}`.
- `lib.rs` `run()`: 3× `.plugin(...)` then `.setup(...)` then `.on_window_event(...)` (CloseRequested→hide) then `.invoke_handler(generate_handler![… commands::usage_snapshot])` then `.build(...).run(cb)`. `Manager` imported. Main window looked up via `app.get_webview_window("main")`.
- `capabilities/default.json`: `windows: ["main"]`, `permissions: ["core:default","opener:default"]`.

---

## Task 1: `api.ts` — usage/autostart wrappers + types

**Files:**
- Modify: `src/api.ts`
- Test: `src/api.test.ts`

**Interfaces:**
- Produces: `UsageSnapshot`/`UsageWindow`/`TokenTotals`/`UsageSource` TS types; `api.usageSnapshot(harness)`, `api.autostartStatus()`, `api.autostartSet(enabled)`, `api.nativeUpdateStatus(critical, lastScanAt?)`.

- [ ] **Step 1: Write the failing test**

First read `src/api.test.ts` to match its existing style (how it mocks `@tauri-apps/api/core` `invoke` and asserts wrappers). Then add tests mirroring that style. If the file mocks `invoke` via `vi.mock('@tauri-apps/api/core', …)` and asserts calls, add:

```ts
  it('usageSnapshot invokes usage_snapshot with the harness', async () => {
    invokeMock.mockResolvedValueOnce({ harness: 'claude', block: {}, week: {}, source: 'local', available: false, generatedAt: '' });
    await api.usageSnapshot('claude');
    expect(invokeMock).toHaveBeenCalledWith('usage_snapshot', { harness: 'claude' });
  });

  it('autostartSet invokes autostart_set with enabled', async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await api.autostartSet(true);
    expect(invokeMock).toHaveBeenCalledWith('autostart_set', { enabled: true });
  });

  it('nativeUpdateStatus invokes native_update_status with critical + lastScanAt', async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await api.nativeUpdateStatus(2, '2026-07-05T00:00:00Z');
    expect(invokeMock).toHaveBeenCalledWith('native_update_status', { critical: 2, lastScanAt: '2026-07-05T00:00:00Z' });
  });
```

(Use whatever the existing file names its invoke mock — `invokeMock` here is illustrative. If the existing tests instead assert `TauriUnavailableError` rejection in jsdom, follow THAT pattern: `await expect(api.usageSnapshot('claude')).rejects.toBeInstanceOf(TauriUnavailableError);`.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd /Users/balakumar/personal/ward && npx vitest run src/api.test.ts`
Expected: FAIL — `api.usageSnapshot is not a function` (or similar).

- [ ] **Step 3: Add the types**

In `src/api.ts`, add a new interface block in the types region (before `export const api`):

```ts
// ── Plan 14 — Usage engine ──────────────────────────────────────────────
export interface TokenTotals {
  input: number;
  output: number;
  cacheCreation: number;
  cacheRead: number;
  total: number;
}

export type UsageSource = 'local' | 'rateLimits';

export interface UsageWindow {
  tokens: TokenTotals;
  costUsd: number;
  percent?: number;      // 0..1 when known (Codex, or Claude w/ configured limit)
  resetsAt?: string;
  resetsInSecs?: number;
  isActive: boolean;
  startedAt?: string;
  planType?: string;
}

export interface UsageSnapshot {
  harness: string;
  block: UsageWindow;    // current 5-hour window
  week: UsageWindow;     // weekly window
  source: UsageSource;
  available: boolean;
  generatedAt: string;
}
```

- [ ] **Step 4: Add the wrappers**

In `src/api.ts`, inside the `export const api = { … }` object (before the closing `};`), add:

```ts
  // Plan 14/15 — usage engine + native shell
  usageSnapshot: (harness: string) => invokeOrThrow<UsageSnapshot>('usage_snapshot', { harness }),
  autostartStatus: () => invokeOrThrow<boolean>('autostart_status'),
  autostartSet: (enabled: boolean) => invokeOrThrow<void>('autostart_set', { enabled }),
  nativeUpdateStatus: (critical: number, lastScanAt?: string) =>
    invokeOrThrow<void>('native_update_status', { critical, lastScanAt }),
```

- [ ] **Step 5: Run tests + typecheck**

Run: `cd /Users/balakumar/personal/ward && npx vitest run src/api.test.ts && npx tsc --noEmit`
Expected: PASS; tsc clean.

- [ ] **Step 6: Commit**

```bash
git add src/api.ts src/api.test.ts
git commit -m "feat(plan15): api.ts usage/autostart/native-status wrappers + types"
```

---

## Task 2: Mock bridge — usage + autostart (dev:mock preview)

Make `usage_snapshot`, `autostart_status`, `autostart_set`, and `native_update_status` answerable in `npm run dev:mock` so the popover can be previewed in a browser.

**Files:**
- Modify: `src/mock/fixtures.ts` (add a `usageSnapshotFor` fixture)
- Modify: `src/mock/store.ts` (add `usageSnapshot`, autostart state, `nativeUpdateStatus` no-op)
- Modify: `src/mock/dispatch.ts` (add the 4 cases)
- Test: `src/mock/store.test.ts` (create if absent, else add) — asserts the store returns a well-formed snapshot and toggles autostart

**Interfaces:**
- Consumes: `UsageSnapshot` (Task 1).
- Produces: mock answers for `usage_snapshot`/`autostart_status`/`autostart_set`/`native_update_status`.

- [ ] **Step 1: Write the failing test**

Create `src/mock/store.test.ts` (or add to it):

```ts
import { describe, it, expect } from 'vitest';
import { MockStore } from './store';

describe('MockStore usage + autostart', () => {
  it('returns a usage snapshot per harness with a 5h block', () => {
    const s = new MockStore();
    const claude = s.usageSnapshot('claude');
    expect(claude.harness).toBe('claude');
    expect(claude.available).toBe(true);
    expect(claude.block.tokens.total).toBeGreaterThan(0);
    const codex = s.usageSnapshot('codex');
    expect(codex.harness).toBe('codex');
    expect(codex.block.percent).toBeGreaterThanOrEqual(0);
  });

  it('toggles autostart state', () => {
    const s = new MockStore();
    const initial = s.autostartStatus();
    s.autostartSet(!initial);
    expect(s.autostartStatus()).toBe(!initial);
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd /Users/balakumar/personal/ward && npx vitest run src/mock/store.test.ts`
Expected: FAIL — `s.usageSnapshot is not a function`.

- [ ] **Step 3: Add the fixture**

In `src/mock/fixtures.ts`, add `import type { UsageSnapshot } from '../api';` to the existing type-import block, then add:

```ts
export function usageSnapshotFor(harness: string): UsageSnapshot {
  const nowSec = 1_780_000_000; // fixed epoch for deterministic mock
  if (harness === 'codex') {
    return {
      harness: 'codex',
      block: {
        tokens: { input: 210_000, output: 12_000, cacheCreation: 0, cacheRead: 188_000, total: 410_000 },
        costUsd: 1.05, percent: 0.31, resetsAt: '2026-07-05T19:00:00Z', resetsInSecs: 9_840,
        isActive: true, startedAt: '2026-07-05T14:00:00Z', planType: 'plus',
      },
      week: {
        tokens: { input: 1_400_000, output: 90_000, cacheCreation: 0, cacheRead: 1_100_000, total: 2_590_000 },
        costUsd: 7.4, percent: 0.17, resetsAt: '2026-07-11T00:00:00Z', resetsInSecs: 500_000,
        isActive: true, startedAt: '2026-07-04T00:00:00Z', planType: 'plus',
      },
      source: 'rateLimits', available: true, generatedAt: '2026-07-05T16:16:00Z',
    };
  }
  return {
    harness: 'claude',
    block: {
      tokens: { input: 820_000, output: 64_000, cacheCreation: 120_000, cacheRead: 240_000, total: 1_244_000 },
      costUsd: 4.18, resetsAt: '2026-07-05T19:00:00Z', resetsInSecs: 9_660,
      isActive: true, startedAt: '2026-07-05T14:00:00Z',
    },
    week: {
      tokens: { input: 12_000_000, output: 900_000, cacheCreation: 1_800_000, cacheRead: 3_700_000, total: 18_400_000 },
      costUsd: 63.2, isActive: true, startedAt: '2026-06-28T00:00:00Z',
    },
    source: 'local', available: true, generatedAt: '2026-07-05T16:16:00Z',
  };
  void nowSec;
}
```

- [ ] **Step 4: Add the store methods**

In `src/mock/store.ts`, add `usageSnapshotFor` to the `./fixtures` import, add a private field `private autostartEnabled = true;` in the class, and add methods:

```ts
  usageSnapshot(harness: string) {
    return clone(usageSnapshotFor(harness));
  }
  autostartStatus(): boolean {
    return this.autostartEnabled;
  }
  autostartSet(enabled: boolean): void {
    this.autostartEnabled = enabled;
  }
  nativeUpdateStatus(): void {
    // no-op in the mock (native badge/tooltip has no browser surface)
  }
```

- [ ] **Step 5: Add the dispatch cases**

In `src/mock/dispatch.ts`, add before the `default:`:

```ts
    // ── Usage engine + native shell (Plan 14/15) ──
    case 'usage_snapshot': await delay(120); return store.usageSnapshot(args.harness ?? 'claude');
    case 'autostart_status': return store.autostartStatus();
    case 'autostart_set': store.autostartSet(!!args.enabled); return null;
    case 'native_update_status': store.nativeUpdateStatus(); return null;
```

- [ ] **Step 6: Run tests + typecheck**

Run: `cd /Users/balakumar/personal/ward && npx vitest run src/mock/store.test.ts && npx tsc --noEmit`
Expected: PASS; tsc clean.

- [ ] **Step 7: Commit**

```bash
git add src/mock/fixtures.ts src/mock/store.ts src/mock/dispatch.ts src/mock/store.test.ts
git commit -m "feat(plan15): mock bridge for usage_snapshot + autostart (dev:mock preview)"
```

---

## Task 3: `<Popover>` component + styling + routing

**Files:**
- Create: `src/entries/Popover.tsx`
- Create: `src/styles/popover.css`
- Modify: `src/index.tsx` (window-label routing)
- Test: `src/entries/Popover.test.tsx`

**Interfaces:**
- Consumes: `api.usageSnapshot`, `api.autostartStatus`, `api.autostartSet`, `isTauri`, `UsageSnapshot`/`UsageWindow` (Task 1).
- Produces: default-exported `Popover` component; `index.tsx` renders it when the window label is `popover` (or `?view=popover` in dev:mock).

- [ ] **Step 1: Write the failing test**

Create `src/entries/Popover.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@solidjs/testing-library';

vi.mock('../api', async () => {
  const actual = await vi.importActual<typeof import('../api')>('../api');
  return {
    ...actual,
    isTauri: () => false,
    api: {
      usageSnapshot: vi.fn((harness: string) => Promise.resolve({
        harness,
        block: {
          tokens: { input: 1_000_000, output: 100_000, cacheCreation: 0, cacheRead: 0, total: 1_100_000 },
          costUsd: 4.5,
          percent: harness === 'codex' ? 0.31 : undefined,
          resetsAt: '2026-07-05T19:00:00Z',
          resetsInSecs: 9_660,
          isActive: true,
          planType: harness === 'codex' ? 'plus' : undefined,
        },
        week: { tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 18_400_000 }, costUsd: 63, isActive: true },
        source: harness === 'codex' ? 'rateLimits' : 'local',
        available: true,
        generatedAt: '2026-07-05T16:16:00Z',
      })),
      autostartStatus: vi.fn(() => Promise.resolve(true)),
      autostartSet: vi.fn(() => Promise.resolve()),
    },
  };
});

import Popover from './Popover';

describe('Popover', () => {
  beforeEach(() => vi.clearAllMocks());

  it('renders both harness rows with usage and a reset countdown', async () => {
    render(() => <Popover />);
    await waitFor(() => expect(screen.getByTestId('pop-harness-claude')).toBeTruthy());
    expect(screen.getByTestId('pop-harness-codex')).toBeTruthy();
    // Codex shows a percent; Claude shows tokens
    await waitFor(() => expect(screen.getByTestId('pop-harness-codex').textContent).toContain('31%'));
    // countdown label present (formatted from resetsInSecs 9660 → "2h 41m")
    expect(screen.getByTestId('pop-harness-claude').textContent).toMatch(/resets/i);
  });

  it('renders the launch-at-login toggle reflecting autostart status', async () => {
    render(() => <Popover />);
    const toggle = await screen.findByTestId('pop-autostart') as HTMLInputElement;
    await waitFor(() => expect(toggle.checked).toBe(true));
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd /Users/balakumar/personal/ward && npx vitest run src/entries/Popover.test.tsx`
Expected: FAIL — cannot resolve `./Popover`.

- [ ] **Step 3: Implement `Popover.tsx`**

Create `src/entries/Popover.tsx`:

```tsx
import { createResource, createSignal, onCleanup, onMount, For, Show } from 'solid-js';
import { api, isTauri, type UsageSnapshot, type UsageWindow } from '../api';
import '../styles/popover.css';

const HARNESSES = [
  { id: 'claude', label: 'Claude Code', icon: '◆' },
  { id: 'codex', label: 'Codex CLI', icon: '◇' },
] as const;

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return `${n}`;
}

function fmtCountdown(secs: number): string {
  if (secs <= 0) return 'resetting…';
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m`;
  return `${secs}s`;
}

async function safeUsage(harness: string): Promise<UsageSnapshot> {
  try {
    return await api.usageSnapshot(harness);
  } catch {
    return {
      harness,
      block: emptyWindow(),
      week: emptyWindow(),
      source: 'local',
      available: false,
      generatedAt: '',
    };
  }
}

function emptyWindow(): UsageWindow {
  return { tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 }, costUsd: 0, isActive: false };
}

function HarnessRow(props: { id: string; label: string; icon: string; snap: UsageSnapshot | undefined; nowMs: number }) {
  const block = () => props.snap?.block;
  // Live countdown: derive remaining from resetsInSecs captured at fetch,
  // minus the seconds elapsed since this component's clock last ticked.
  const remaining = () => {
    const w = block();
    if (!w || w.resetsInSecs == null) return null;
    return Math.max(0, w.resetsInSecs - Math.floor((props.nowMs - startedAtRef) / 1000));
  };
  let startedAtRef = props.nowMs;
  const pct = () => {
    const p = block()?.percent;
    return p == null ? null : Math.round(p * 100);
  };
  const ramp = () => {
    const p = block()?.percent ?? 0;
    return p >= 0.9 ? 'crit' : p >= 0.7 ? 'warn' : 'ok';
  };
  return (
    <div class="pop-harness" data-testid={`pop-harness-${props.id}`}>
      <div class="pop-harness-head">
        <span class="pop-harness-icon">{props.icon}</span>
        <span class="pop-harness-name">{props.label}</span>
        <Show when={block()?.planType}>{(pt) => <span class="pop-plan">{pt()}</span>}</Show>
      </div>
      <Show
        when={props.snap?.available}
        fallback={<div class="pop-empty">No usage found</div>}
      >
        <Show
          when={pct() != null}
          fallback={
            <div class="pop-metric">
              <span class="pop-tokens">{fmtTokens(block()?.tokens.total ?? 0)} tok</span>
              <span class="pop-cost">${(block()?.costUsd ?? 0).toFixed(2)}</span>
            </div>
          }
        >
          <div class={`pop-gauge pop-gauge-${ramp()}`}>
            <div class="pop-gauge-fill" style={{ width: `${pct()}%` }} />
            <span class="pop-gauge-label">{pct()}%</span>
          </div>
        </Show>
        <Show when={remaining() != null}>
          <div class="pop-reset">resets in {fmtCountdown(remaining()!)}</div>
        </Show>
      </Show>
    </div>
  );
}

export default function Popover() {
  const [nowMs, setNowMs] = createSignal(Date.now());
  const [claude, { refetch: refetchClaude }] = createResource(() => safeUsage('claude'));
  const [codex, { refetch: refetchCodex }] = createResource(() => safeUsage('codex'));
  const [autostart, setAutostart] = createSignal<boolean>(false);

  const tick = setInterval(() => setNowMs(Date.now()), 1000);
  onCleanup(() => clearInterval(tick));

  const poll = setInterval(() => refetchAll(), 15000);
  onCleanup(() => clearInterval(poll));

  function refetchAll() {
    void refetchClaude();
    void refetchCodex();
  }

  onMount(async () => {
    try {
      setAutostart(await api.autostartStatus());
    } catch {
      setAutostart(false);
    }
    if (isTauri()) {
      try {
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        const un = await getCurrentWindow().onFocusChanged(({ payload }) => {
          if (payload) refetchAll();
        });
        onCleanup(un);
      } catch {
        /* window API unavailable — poll covers refresh */
      }
    }
  });

  async function toggleAutostart(e: Event) {
    const next = (e.currentTarget as HTMLInputElement).checked;
    try {
      await api.autostartSet(next);
      setAutostart(next);
    } catch {
      setAutostart(!next); // revert on failure
    }
  }

  async function fire(event: string, payload?: unknown) {
    try {
      const { emit } = await import('@tauri-apps/api/event');
      await emit(event, payload);
    } catch {
      /* dev:mock / non-Tauri — no-op */
    }
  }

  return (
    <div class="pop" data-testid="popover">
      <header class="pop-head">
        <span class="pop-title">Ward</span>
        <button class="pop-refresh" title="Refresh" onClick={() => refetchAll()} data-testid="pop-refresh">⟳</button>
      </header>
      <For each={HARNESSES}>
        {(h) => (
          <HarnessRow
            id={h.id}
            label={h.label}
            icon={h.icon}
            snap={h.id === 'claude' ? claude() : codex()}
            nowMs={nowMs()}
          />
        )}
      </For>
      <footer class="pop-foot">
        <div class="pop-actions">
          <button class="pop-btn" onClick={() => fire('scan-now')} data-testid="pop-scan">Scan now</button>
          <button class="pop-btn" onClick={() => fire('tray_action', 'open')} data-testid="pop-open">Open</button>
        </div>
        <label class="pop-toggle">
          <input type="checkbox" checked={autostart()} onChange={toggleAutostart} data-testid="pop-autostart" />
          <span>Launch at login</span>
        </label>
      </footer>
    </div>
  );
}
```

- [ ] **Step 4: Implement `popover.css`**

Create `src/styles/popover.css`:

```css
.pop {
  font-family: var(--font-ui);
  background: var(--bg);
  color: var(--text);
  width: 100%;
  min-height: 100vh;
  padding: 12px;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  gap: 10px;
  -webkit-user-select: none;
  user-select: none;
}
.pop-head { display: flex; align-items: center; justify-content: space-between; }
.pop-title { font-weight: 600; letter-spacing: 0.02em; color: var(--text); }
.pop-refresh {
  background: transparent; border: none; color: var(--text-dim);
  font-size: 15px; cursor: pointer; border-radius: var(--r-sm); padding: 2px 6px;
  transition: color 120ms ease, background 120ms ease;
}
.pop-refresh:hover { color: var(--accent); background: var(--accent-bg-2); }

.pop-harness {
  background: var(--surface); border: 1px solid var(--border);
  border-radius: var(--r-md); padding: 10px; display: flex; flex-direction: column; gap: 8px;
}
.pop-harness-head { display: flex; align-items: center; gap: 8px; }
.pop-harness-icon { color: var(--accent); }
.pop-harness-name { font-size: 13px; font-weight: 500; flex: 1; }
.pop-plan {
  font-size: 10px; text-transform: uppercase; letter-spacing: 0.04em;
  color: var(--text-dim); border: 1px solid var(--border); border-radius: var(--r-pill); padding: 1px 7px;
}
.pop-empty { font-size: 12px; color: var(--text-mute); }

.pop-gauge {
  position: relative; height: 20px; border-radius: var(--r-pill);
  background: var(--surface-3); overflow: hidden; display: flex; align-items: center;
}
.pop-gauge-fill { position: absolute; left: 0; top: 0; bottom: 0; transition: width 400ms ease; }
.pop-gauge-ok .pop-gauge-fill { background: linear-gradient(90deg, var(--accent-2), var(--accent)); }
.pop-gauge-warn .pop-gauge-fill { background: var(--warn); }
.pop-gauge-crit .pop-gauge-fill { background: var(--crit); }
.pop-gauge-label { position: relative; margin-left: auto; margin-right: 8px; font-size: 11px; font-variant-numeric: tabular-nums; }

.pop-metric { display: flex; align-items: baseline; justify-content: space-between; }
.pop-tokens { font-size: 13px; font-variant-numeric: tabular-nums; }
.pop-cost { font-size: 12px; color: var(--text-dim); font-variant-numeric: tabular-nums; }
.pop-reset { font-size: 11px; color: var(--text-dim); font-variant-numeric: tabular-nums; }

.pop-foot { display: flex; flex-direction: column; gap: 8px; margin-top: auto; }
.pop-actions { display: flex; gap: 8px; }
.pop-btn {
  flex: 1; background: var(--surface-3); color: var(--text);
  border: 1px solid var(--border); border-radius: var(--r-sm); padding: 6px 8px;
  font-size: 12px; cursor: pointer; transition: background 120ms ease, border-color 120ms ease;
}
.pop-btn:hover { background: var(--surface-4); border-color: var(--border-strong); }
.pop-toggle { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--text-dim); cursor: pointer; }
.pop-toggle input { accent-color: var(--accent); }
```

- [ ] **Step 5: Add window-label routing to `index.tsx`**

Replace `src/index.tsx` with:

```tsx
/* @refresh reload */
import "./styles/tokens.css";
import "./styles/app.css";
import { render } from "solid-js/web";
import App from "./App";
import Popover from "./entries/Popover";

/** True when this webview is the tray popover window (native label === 'popover'),
 *  or, in dev:mock browser preview, when the URL carries ?view=popover. */
function isPopoverWindow(): boolean {
  if (new URLSearchParams(window.location.search).get("view") === "popover") return true;
  const internals = (globalThis as { __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } } }).__TAURI_INTERNALS__;
  return internals?.metadata?.currentWindow?.label === "popover";
}

async function boot() {
  if (import.meta.env.VITE_WARD_MOCK) {
    await import("./mock/install");
  }
  const root = document.getElementById("root") as HTMLElement;
  if (isPopoverWindow()) {
    render(() => <Popover />, root);
  } else {
    render(() => <App />, root);
  }
}

void boot();
```

- [ ] **Step 6: Run tests + typecheck**

Run: `cd /Users/balakumar/personal/ward && npx vitest run src/entries/Popover.test.tsx && npx tsc --noEmit`
Expected: PASS; tsc clean. (If the countdown assertion is brittle under timers, the component derives `remaining()` from `resetsInSecs` at first render — the test asserts the `resets` label text, not an exact value.)

- [ ] **Step 7: Full frontend test run**

Run: `cd /Users/balakumar/personal/ward && npm test`
Expected: all vitest suites PASS (existing + new).

- [ ] **Step 8: Commit**

```bash
git add src/entries/Popover.tsx src/styles/popover.css src/index.tsx src/entries/Popover.test.tsx
git commit -m "feat(plan15): glance popover component + styling + window-label routing"
```

---

## Task 4: Native popover window + positioner + tray toggle

Create the hidden `popover` webview window, register `tauri-plugin-positioner`, and toggle/anchor the popover under the tray on left-click; hide it on blur. Keep the main window's close-to-tray intact.

**Files:**
- Modify: `src-tauri/Cargo.toml` (positioner dep)
- Modify: `src-tauri/src/lib.rs` (plugin, popover window, scoped window events)
- Modify: `src-tauri/src/native/tray.rs` (left-click → toggle popover via positioner)
- Modify: `src-tauri/capabilities/default.json` (popover window + positioner perms)

**Interfaces:**
- Consumes: the `popover` frontend route (Task 3).
- Produces: a tray-anchored popover window toggled by left-click, hidden on blur.

- [ ] **Step 1: Add the dependency**

In `src-tauri/Cargo.toml` `[dependencies]`, after the autostart line:

```toml
# Plan 15 — tray-anchored popover positioning
tauri-plugin-positioner = { version = "2", features = ["tray-icon"] }
```

- [ ] **Step 2: Register the plugin + create the hidden popover window**

In `src-tauri/src/lib.rs`:

(a) Add the plugin among the `.plugin(...)` calls (after the autostart plugin):

```rust
        .plugin(tauri_plugin_positioner::init())
```

(b) Inside `.setup(|app| { … })`, after the tray-setup `match` block (and before the `--start-hidden` block), create the hidden popover window:

```rust
            // Glance popover window (Plan 15): a small decorationless webview
            // that loads the same bundle; `index.tsx` renders <Popover> for the
            // "popover" label. Hidden until the tray icon is left-clicked.
            match tauri::WebviewWindowBuilder::new(
                app,
                "popover",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Ward")
            .inner_size(320.0, 300.0)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build()
            {
                Ok(_) => {}
                Err(e) => eprintln!("ward: popover window setup failed: {e}"),
            }
```

- [ ] **Step 3: Scope the window-event handler (blur-hide for popover)**

In `src-tauri/src/lib.rs`, replace the existing `.on_window_event(|window, event| { … })` block with:

```rust
        .on_window_event(|window, event| {
            match event {
                // Close-to-tray (Plan 13): red button / ⌘W hides; only a real quit closes.
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if crate::native::lifecycle::should_hide_on_close(
                        crate::native::lifecycle::is_quitting(),
                    ) {
                        let _ = window.hide();
                        api.prevent_close();
                    }
                }
                // Popover (Plan 15): hide when it loses focus, like a native menu-bar popover.
                tauri::WindowEvent::Focused(false) if window.label() == "popover" => {
                    let _ = window.hide();
                }
                _ => {}
            }
        })
```

- [ ] **Step 4: Toggle the popover from the tray left-click**

In `src-tauri/src/native/tray.rs`:

(a) Add `MouseButton` to the tray import:

```rust
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
```

(b) Replace the `.on_tray_icon_event(...)` closure with one that forwards positioner events and toggles the popover on left-click-up:

```rust
        .on_tray_icon_event(|tray, event| {
            // Let the positioner cache the tray-icon rect for TrayCenter anchoring.
            tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
            if let TrayIconEvent::Click { button, button_state, .. } = event {
                if matches!(button, MouseButton::Left) && matches!(button_state, MouseButtonState::Up) {
                    let app = tray.app_handle();
                    if let Some(win) = app.get_webview_window("popover") {
                        if win.is_visible().unwrap_or(false) {
                            let _ = win.hide();
                        } else {
                            use tauri_plugin_positioner::{Position, WindowExt};
                            let _ = win.move_window(Position::TrayCenter);
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                }
            }
        })
```

- [ ] **Step 5: Grant the popover window + positioner capabilities**

Replace `src-tauri/capabilities/default.json` with:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the main and popover windows",
  "windows": ["main", "popover"],
  "permissions": [
    "core:default",
    "opener:default",
    "positioner:default"
  ]
}
```

- [ ] **Step 6: Build + typecheck**

Run: `cd /Users/balakumar/personal/ward/src-tauri && cargo check`
Expected: no errors. (If `positioner:default` is not a recognized permission for the installed plugin version, consult the generated `gen/schemas` and use the plugin's actual default permission identifier; if the `move_window`/`Position`/`on_tray_event` API differs, adapt to the current `tauri-plugin-positioner` v2 API — behavior must stay: forward tray events + anchor at TrayCenter.)

- [ ] **Step 7: Run the Rust suite (tray/lifecycle unaffected)**

Run: `cd /Users/balakumar/personal/ward/src-tauri && cargo test --lib native::`
Expected: PASS (existing tray/lifecycle/watch tests unaffected).

- [ ] **Step 8: Commit (with Cargo.lock)**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/src/native/tray.rs src-tauri/capabilities/default.json
git commit -m "feat(plan15): tray-anchored popover window + positioner + blur-hide"
```

---

## Task 5: `native_update_status` command + Security wiring (badge + tooltip)

Drive the dock badge + tray tooltip from the latest security scan's critical count.

**Files:**
- Modify: `src-tauri/src/commands.rs` (add `native_update_status`)
- Modify: `src-tauri/src/lib.rs` (register it)
- Modify: `src/modes/Security.tsx` (call it after each scan)
- Test: `src-tauri/src/native/tray.rs` (a `count`→badge-payload test already exists; add a tooltip-format assertion if not covered) + `src/modes/Security.test.tsx` (assert the call after scan)

**Interfaces:**
- Consumes: `tray::update_badge`, `tray::format_tooltip` (exist); `api.nativeUpdateStatus` (Task 1).
- Produces: `native_update_status(app, critical, lastScanAt?)` command; Security calls it after each scan.

- [ ] **Step 1: Write the failing Rust test**

In `src-tauri/src/native/tray.rs` tests, add (the badge payload + tooltip formatting are already unit-tested; this pins the tooltip includes the scan time path used by the command):

```rust
    #[test]
    fn format_tooltip_zero_critical_no_scan() {
        assert_eq!(format_tooltip(0, None), "Ward — 0 critical");
    }
```

(If an equivalent assertion already exists, skip adding a duplicate and note it.)

- [ ] **Step 2: Run to verify (this may already pass — the real deliverable is the command)**

Run: `cd /Users/balakumar/personal/ward/src-tauri && cargo test --lib native::tray::tests`
Expected: PASS (formatter is pure). Proceed to wire the command.

- [ ] **Step 3: Add the command**

In `src-tauri/src/commands.rs`, add `use tauri::Manager;` if not already imported at the top (needed for `app.tray_by_id`), then add near the other commands:

```rust
/// Plan 15 — push the latest scan's critical count to the dock badge + tray tooltip.
#[tauri::command]
pub fn native_update_status(
    app: tauri::AppHandle,
    critical: usize,
    last_scan_at: Option<String>,
) -> Result<(), WardError> {
    crate::native::tray::update_badge(&app, critical);
    if let Some(tray) = app.tray_by_id("ward-tray") {
        let tip = crate::native::tray::format_tooltip(critical, last_scan_at.as_deref());
        let _ = tray.set_tooltip(Some(tip));
    }
    Ok(())
}
```

- [ ] **Step 4: Register the command**

In `src-tauri/src/lib.rs` `generate_handler![ … ]`, add after `commands::usage_snapshot` (add a trailing comma to it):

```rust
            commands::usage_snapshot,
            commands::native_update_status
```

- [ ] **Step 5: Call it from Security after each scan**

In `src/modes/Security.tsx`, add a `createEffect` (import `createEffect` from `solid-js` if not already) that fires when the scan resolves, pushing the critical count. Place it near the existing `listen(...)` setup:

```tsx
  createEffect(() => {
    const r = scan();
    if (r && isTauri()) {
      void api.nativeUpdateStatus(r.severityCounts.critical, r.timestamp);
    }
  });
```

Ensure `isTauri` and `api` are imported from `../api` (api is already used; add `isTauri` to the import if missing). This is a no-op in dev:mock's browser (the mock answers `native_update_status` with null).

- [ ] **Step 6: Write/adjust the Security test**

In `src/modes/Security.test.tsx`, add a test that after the scan resolves, `api.nativeUpdateStatus` is called with the critical count. Follow the file's existing mock-of-`api` pattern; assert:

```tsx
  it('pushes critical count to the native badge after a scan', async () => {
    // (mock api.securityScan to resolve severityCounts.critical = 1, isTauri → true)
    // render <Security .../>, then:
    await waitFor(() => expect(nativeUpdateStatusMock).toHaveBeenCalledWith(1, expect.any(String)));
  });
```

(Match the existing test's harness — how it mocks `api` and `isTauri`, and how it passes `items`. If the existing Security tests mock `@tauri-apps/api/event` `listen`, keep that mock so `createEffect` runs without a real Tauri context.)

- [ ] **Step 7: Run all tests + typecheck**

Run: `cd /Users/balakumar/personal/ward && npm test && npx tsc --noEmit && cd src-tauri && cargo test --lib && cargo check`
Expected: all PASS; tsc + cargo check clean.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs src-tauri/src/native/tray.rs src/modes/Security.tsx src/modes/Security.test.tsx
git commit -m "feat(plan15): native_update_status — dock badge + tray tooltip from scan"
```

---

## Hands-on verification (user, after all tasks)

macOS can't run `tauri-driver`, so verify the popover two ways:

1. **Browser preview (`dev:mock`):** `npm run dev:mock`, open `http://localhost:1430/?view=popover` in Chrome — the popover renders on fixture data (Claude tokens/cost + reset, Codex 31% gauge + reset, launch-at-login toggle). Toggle + buttons are inert in the browser (no native events).
2. **Real app (`npm run tauri dev`):** left-click the tray icon → the popover appears anchored under it, showing live Claude/Codex usage + reset countdowns; click away → it hides (blur). Toggle "Launch at login" and confirm it flips the login item. Run a Security scan → the dock badge + tray tooltip reflect the critical count.

---

## Self-Review

**Spec coverage (design §7.3–§8, plus §7.4 badge/tooltip moved here from Plan 13):**
- Glance popover with usage gauges + reset countdowns (§8) → Tasks 1–3 ✓
- Refresh model — 1s countdown tick, fetch-on-focus, poll-while-mounted (§8 refresh) → Task 3 ✓
- Tray-anchored popover window via positioner, blur-hide (§7.3) → Task 4 ✓
- Window-label routing + `?view=popover` dev fallback (§8) → Task 3 ✓
- Launch-at-login toggle in the popover (§7.2) → Task 3 ✓
- dock badge + tray tooltip from scan (§7.4, moved from Plan 13) → Task 5 ✓
- dev:mock fixtures (§11) → Task 2 ✓
- **Documented deviations:** (a) critical findings surface on the **dock badge + tray tooltip** (Task 5), NOT inside the popover, so opening the popover never triggers a heavy security scan; (b) popover default height 320×300 (compact — two harness rows + footer), positioner anchors at `TrayCenter`.

**Placeholder scan:** no `TODO`/`TBD`. Each task's tests reference the file's existing mock pattern where the exact harness (invoke mock vs rejection) depends on the current test file — the implementer aligns to it (called out explicitly, not left vague).

**Type consistency:** `UsageSnapshot`/`UsageWindow`/`TokenTotals`/`UsageSource` (Task 1) match the Rust `usage::mod` structs and are consumed unchanged by `Popover.tsx` (Task 3) and the mock (Task 2). `api.usageSnapshot`/`autostartStatus`/`autostartSet`/`nativeUpdateStatus` names match between `api.ts` (Task 1), the mock dispatch (Task 2), `Popover.tsx` (Task 3), and `Security.tsx` (Task 5). Command `native_update_status` matches between `commands.rs`, `generate_handler!` (Task 5), and the `api.ts` wrapper (Task 1). Window label `"popover"` matches between `index.tsx` routing (Task 3), the window builder + event scoping (Task 4), the tray toggle (Task 4), and the capability `windows` list (Task 4).
