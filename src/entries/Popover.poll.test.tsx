import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, waitFor } from '@solidjs/testing-library';

// Controllable focus callback so the test can drive focus transitions.
let focusCb: ((e: { payload: boolean }) => void) | null = null;

vi.mock('../api', async () => {
  const actual = await vi.importActual<typeof import('../api')>('../api');
  const emptySnap = (harness: string) => ({
    harness,
    block: { tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 }, costUsd: 0, isActive: false },
    week: { tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 }, costUsd: 0, isActive: false },
    source: 'local' as const,
    available: true,
    generatedAt: '',
  });
  return {
    ...actual,
    isTauri: () => true,
    api: {
      // Codex is the local probe we count. Claude is left opted-out
      // (liveUsageEnabled=false) so it stays in opt-in mode and issues no fetch.
      usageSnapshot: vi.fn((harness: string) => Promise.resolve(emptySnap(harness))),
      usageCached: vi.fn(() => Promise.resolve(null)),
      liveUsageEnabled: vi.fn(() => Promise.resolve(false)),
      autostartStatus: vi.fn(() => Promise.resolve(false)),
      autostartSet: vi.fn(() => Promise.resolve()),
    },
  };
});

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onFocusChanged: (cb: (e: { payload: boolean }) => void) => {
      focusCb = cb;
      return Promise.resolve(() => {});
    },
  }),
}));

import Popover from './Popover';
import { api } from '../api';

const calls = () => (api.usageSnapshot as unknown as ReturnType<typeof vi.fn>).mock.calls.length;

describe('Popover refresh model', () => {
  beforeEach(() => {
    focusCb = null;
    vi.clearAllMocks();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  // Authentic contract after Plan 16: there is NO silent background poll (it
  // would fire the gated live network call on a timer). The popover fetches on
  // mount and again whenever the window regains focus (i.e. is re-opened from
  // the tray). We enable fake timers only AFTER the async mount settles so the
  // dynamic-import / focus-handler promise chain resolves under real timers.
  it('fetches on mount and on focus regain, never on a silent timer', async () => {
    render(() => <Popover />);

    // Mount settles: focus handler registered + initial codex fetch.
    await waitFor(() => expect(focusCb).toBeTypeOf('function'));
    await waitFor(() => expect(calls()).toBeGreaterThanOrEqual(1));

    vi.useFakeTimers();
    const base = calls();

    // No focus change: advancing a full minute must trigger ZERO fetches — the
    // 1s countdown tick fires but never reads usage; there is no poll interval.
    await vi.advanceTimersByTimeAsync(60_000);
    expect(calls()).toBe(base);

    // Focus regain (popover re-opened) → exactly one refetch round.
    focusCb!({ payload: true });
    await vi.advanceTimersByTimeAsync(0); // flush the refetch's promises
    const afterFocus = calls();
    expect(afterFocus).toBeGreaterThan(base);

    // Still no silent poll after the focus-driven refetch.
    await vi.advanceTimersByTimeAsync(60_000);
    expect(calls()).toBe(afterFocus);
  });
});
