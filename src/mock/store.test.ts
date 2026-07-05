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

  it('exposes a claude-only live snapshot, toggleable via the opt-in flag', () => {
    const s = new MockStore();
    expect(s.liveUsageEnabled()).toBe(true); // dev:mock defaults on
    const live = s.usageSnapshotLive('claude');
    expect(live.source).toBe('live');
    expect(live.block.percent).toBeGreaterThan(0);
    expect(live.week.percent).toBeGreaterThan(0);
    expect(() => s.usageSnapshotLive('codex')).toThrow(); // live is Claude-only
    s.setLiveUsageEnabled(false);
    expect(s.liveUsageEnabled()).toBe(false);
  });
});
