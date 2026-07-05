import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, waitFor } from '@solidjs/testing-library';

// Controllable focus callback so the test can drive blur/focus transitions.
let focusCb: ((e: { payload: boolean }) => void) | null = null;

vi.mock('../api', async () => {
  const actual = await vi.importActual<typeof import('../api')>('../api');
  return {
    ...actual,
    // Run the Tauri focus-gated code path (the other Popover test uses false).
    isTauri: () => true,
    api: {
      usageSnapshot: vi.fn((harness: string) =>
        Promise.resolve({
          harness,
          block: {
            tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 },
            costUsd: 0,
            resetsAt: '2026-07-05T19:00:00Z',
            resetsInSecs: 9_660,
            isActive: true,
          },
          week: { tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 }, costUsd: 0, isActive: false },
          source: 'local',
          available: true,
          generatedAt: '2026-07-05T16:16:00Z',
        }),
      ),
      autostartStatus: vi.fn(() => Promise.resolve(false)),
      autostartSet: vi.fn(() => Promise.resolve()),
    },
  };
});

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    isFocused: () => Promise.resolve(true),
    onFocusChanged: (cb: (e: { payload: boolean }) => void) => {
      focusCb = cb;
      return Promise.resolve(() => {});
    },
  }),
}));

import Popover from './Popover';
import { api } from '../api';

const calls = () => (api.usageSnapshot as unknown as ReturnType<typeof vi.fn>).mock.calls.length;

describe('Popover poll focus-gating', () => {
  beforeEach(() => {
    focusCb = null;
    vi.clearAllMocks();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  // Authentic contract: the 20s poll must run ONLY while the popover window is
  // focused. We enable fake timers AFTER the async mount settles so the mount's
  // dynamic-import / isFocused() promise chain resolves under real timers, then
  // clear the mount's (real-timer) poll via a blur first — so EVERY interval we
  // assert on below is created under fake timers and is genuinely advanceable.
  it('polls while focused, stops on blur, resumes on focus', async () => {
    render(() => <Popover />);

    // Mount settles: focus handler registered + initial createResource fetch
    // (claude + codex => 2 usageSnapshot calls). isFocused()===true armed a poll.
    await waitFor(() => expect(focusCb).toBeTypeOf('function'));
    await waitFor(() => expect(calls()).toBeGreaterThanOrEqual(2));

    // Blur clears the mount's real-timer poll so it cannot leak calls.
    focusCb!({ payload: false });

    // From here, every setInterval/clearInterval is faked and controllable.
    vi.useFakeTimers();
    const base = calls();

    // Focus → resume: immediate refetchAll() (+2) AND a fresh 20s poll (faked).
    focusCb!({ payload: true });
    await vi.advanceTimersByTimeAsync(0); // flush the synchronous refetch's promises
    const afterFocus = calls();
    expect(afterFocus).toBeGreaterThan(base); // focus genuinely resumed fetching

    // While focused, advancing one 20s interval fires the poll (refetch => +2).
    await vi.advanceTimersByTimeAsync(20_000);
    const afterPoll = calls();
    expect(afterPoll).toBeGreaterThan(afterFocus); // the poll actually ran

    // Blur → stop. Advancing 60s (three intervals) yields ZERO further calls.
    focusCb!({ payload: false });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(calls()).toBe(afterPoll); // poll cleared: no disk reads while hidden
  });
});
