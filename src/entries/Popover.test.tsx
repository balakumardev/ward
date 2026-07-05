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
