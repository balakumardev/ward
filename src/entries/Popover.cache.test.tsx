import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@solidjs/testing-library';

// Plan 17 — cache-first paint (stale-while-revalidate). The popover reads the
// last-known snapshot from `api.usageCached` on mount and paints it instantly,
// then the normal resource fetch refreshes it in the background. We drive Codex
// (no live opt-in in the way) and hold the fresh refresh on a deferred promise
// so we can assert the CACHED gauge shows first, then the FRESH one swaps in.
const h = vi.hoisted(() => {
  const emptyTokens = { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 };
  const cachedCodex = {
    harness: 'codex',
    block: { tokens: { ...emptyTokens }, costUsd: 0, percent: 0.12, resetsAt: '2026-07-05T19:00:00Z', isActive: true, planType: 'plus' },
    week: { tokens: { ...emptyTokens }, costUsd: 0, isActive: false },
    source: 'rateLimits', available: true, generatedAt: '',
  };
  const freshCodex = { ...cachedCodex, block: { ...cachedCodex.block, percent: 0.58 } };
  const claudeSnap = {
    harness: 'claude',
    block: { tokens: { ...emptyTokens }, costUsd: 0, isActive: false },
    week: { tokens: { ...emptyTokens }, costUsd: 0, isActive: false },
    source: 'local', available: false, generatedAt: '',
  };
  const deferred: { resolve: (v: unknown) => void; promise: Promise<unknown> } = {
    resolve: () => {},
    promise: Promise.resolve(),
  };
  const resetDeferred = () => {
    deferred.promise = new Promise((r) => { deferred.resolve = r as (v: unknown) => void; });
  };
  resetDeferred();
  return { cachedCodex, freshCodex, claudeSnap, deferred, resetDeferred };
});

vi.mock('../api', async () => {
  const actual = await vi.importActual<typeof import('../api')>('../api');
  return {
    ...actual,
    isTauri: () => false,
    api: {
      // Codex's background refresh is held on the deferred; Claude resolves
      // immediately (unavailable) and is irrelevant to these assertions.
      usageSnapshot: vi.fn((harness: string) =>
        harness === 'codex' ? h.deferred.promise : Promise.resolve(h.claudeSnap)),
      // The warm cache: Codex has a last-known snapshot, Claude has nothing.
      usageCached: vi.fn((harness: string) =>
        Promise.resolve(harness === 'codex' ? h.cachedCodex : null)),
      autostartStatus: vi.fn(() => Promise.resolve(false)),
      autostartSet: vi.fn(() => Promise.resolve()),
    },
  };
});

import Popover from './Popover';

describe('Popover cache-first paint', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    h.resetDeferred();
  });

  it('paints the cached gauge on mount, before the background refresh resolves', async () => {
    render(() => <Popover />);
    const row = await screen.findByTestId('pop-harness-codex');
    // The cached 12% gauge appears while the fresh refresh is still pending.
    await waitFor(() => expect(row.textContent).toContain('12%'));
    expect(row.textContent).not.toContain('58%'); // fresh not resolved yet
  });

  it('replaces the cached gauge with the fresh value when the refresh resolves', async () => {
    render(() => <Popover />);
    const row = await screen.findByTestId('pop-harness-codex');
    await waitFor(() => expect(row.textContent).toContain('12%')); // cached first
    // Resolve the background refresh → the fresh 58% swaps in over the cached 12%.
    h.deferred.resolve(h.freshCodex);
    await waitFor(() => expect(row.textContent).toContain('58%'));
  });
});
