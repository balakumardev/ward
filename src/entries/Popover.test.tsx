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

import Popover, { secsUntil, clampPopoverHeight, POPOVER_MIN_H, POPOVER_MAX_H } from './Popover';

describe('clampPopoverHeight', () => {
  it('clamps to [MIN, MAX] and ceils fractional heights', () => {
    expect(clampPopoverHeight(420.2)).toBe(421); // mid → ceil
    expect(clampPopoverHeight(POPOVER_MAX_H + 250)).toBe(POPOVER_MAX_H); // tall content → clamp + internal scroll
    expect(clampPopoverHeight(10)).toBe(POPOVER_MIN_H); // tiny → floor
  });

  it('falls back to MIN for non-finite / non-positive input (jsdom scrollHeight === 0)', () => {
    expect(clampPopoverHeight(0)).toBe(POPOVER_MIN_H);
    expect(clampPopoverHeight(-5)).toBe(POPOVER_MIN_H);
    expect(clampPopoverHeight(Number.NaN)).toBe(POPOVER_MIN_H);
  });
});

describe('Popover', () => {
  beforeEach(() => vi.clearAllMocks());

  it('secsUntil ticks down and floors at zero', () => {
    const iso = '2026-07-05T19:00:00Z';
    const t0 = Date.parse('2026-07-05T16:19:00Z'); // 2h41m before
    const a = secsUntil(iso, t0);
    const b = secsUntil(iso, t0 + 60_000);
    expect(a).toBeGreaterThan(b!);
    expect(b).toBe(a! - 60);
    expect(secsUntil(iso, Date.parse('2026-07-05T20:00:00Z'))).toBe(0); // past → 0
    expect(secsUntil(undefined, t0)).toBeNull();
  });

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
