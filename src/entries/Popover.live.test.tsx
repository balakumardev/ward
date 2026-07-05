import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@solidjs/testing-library';

// Hoisted so the (hoisted) vi.mock factory can reference these safely.
const h = vi.hoisted(() => ({
  usageSnapshotLive: vi.fn(),
  setLiveUsageEnabled: vi.fn(() => Promise.resolve()),
  liveEnabledValue: true,
}));
const usageSnapshotLive = h.usageSnapshotLive;
const setLiveUsageEnabled = h.setLiveUsageEnabled;

vi.mock('../api', async () => {
  const actual = await vi.importActual<typeof import('../api')>('../api');
  const localSnap = (harness: string) => ({
    harness,
    block: { tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 }, costUsd: 0, isActive: false },
    week: { tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 }, costUsd: 0, isActive: false },
    source: 'local' as const,
    available: false,
    generatedAt: '',
  });
  return {
    ...actual,
    isTauri: () => true,
    api: {
      usageSnapshot: vi.fn((harness: string) => Promise.resolve(localSnap(harness))),
      usageCached: vi.fn(() => Promise.resolve(null)),
      usageSnapshotLive: h.usageSnapshotLive,
      liveUsageEnabled: vi.fn(() => Promise.resolve(h.liveEnabledValue)),
      setLiveUsageEnabled: h.setLiveUsageEnabled,
      autostartStatus: vi.fn(() => Promise.resolve(false)),
      autostartSet: vi.fn(() => Promise.resolve()),
    },
  };
});

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged: () => Promise.resolve(() => {}) }),
}));

import Popover from './Popover';

const liveSnap = {
  harness: 'claude',
  block: {
    tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 },
    costUsd: 0, percent: 0.26, resetsAt: '2026-07-05T19:30:00Z', isActive: true, planType: 'max',
  },
  week: {
    tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 },
    costUsd: 0, percent: 0.44, resetsAt: '2026-07-09T00:00:00Z', isActive: true, planType: 'max',
  },
  source: 'live' as const,
  available: true,
  generatedAt: '',
};

describe('Popover live Claude usage', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    h.liveEnabledValue = true;
  });

  it('renders live 5h and weekly gauges when opted in', async () => {
    usageSnapshotLive.mockResolvedValue(liveSnap);
    render(() => <Popover />);
    const row = await screen.findByTestId('pop-harness-claude');
    await waitFor(() => expect(row.textContent).toContain('26%')); // 5h gauge
    expect(row.textContent).toContain('44%'); // weekly gauge
    expect(row.textContent).toMatch(/live/i); // source label
    expect(row.textContent).toContain('max'); // plan chip
    expect(usageSnapshotLive).toHaveBeenCalledWith('claude');
  });

  it('shows the opt-in button when not enabled, and fetches live on enable', async () => {
    h.liveEnabledValue = false;
    usageSnapshotLive.mockResolvedValue(liveSnap);
    render(() => <Popover />);
    const btn = await screen.findByTestId('pop-enable-live');
    expect(usageSnapshotLive).not.toHaveBeenCalled(); // no network before opt-in
    fireEvent.click(btn);
    await waitFor(() => expect(setLiveUsageEnabled).toHaveBeenCalledWith(true));
    await waitFor(() => expect(screen.getByTestId('pop-harness-claude').textContent).toContain('26%'));
  });

  it('shows an error + Retry when the live fetch fails, and recovers on retry', async () => {
    usageSnapshotLive.mockRejectedValueOnce({ kind: 'live', message: 'live usage error: Claude login has expired.' });
    render(() => <Popover />);
    const err = await screen.findByTestId('pop-live-error');
    expect(err.textContent).toContain('expired');
    expect(err.textContent).not.toContain('live usage error:'); // internal prefix stripped

    usageSnapshotLive.mockResolvedValueOnce(liveSnap);
    fireEvent.click(screen.getByTestId('pop-live-retry'));
    await waitFor(() => expect(screen.getByTestId('pop-harness-claude').textContent).toContain('26%'));
  });
});
