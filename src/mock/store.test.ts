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

describe('MockStore MCP upsert', () => {
  it('upsertMcpEntry edits an existing MCP item config in place', () => {
    const s = new MockStore();
    const before = s.scan('claude').items.find((i) => i.category === 'mcp')!;
    const r = s.upsertMcpEntry('claude', before.scopeId, before.name, { command: 'edited', args: ['z'] }, before.path);
    expect(r.kind).toBe('mcp-upsert');
    const after = s.scan('claude').items.find((i) => i.category === 'mcp' && i.name === before.name)!;
    expect((after.mcpConfig as { command: string }).command).toBe('edited');
  });

  it('upsertMcpEntry adds a new MCP item and undo removes it', () => {
    const s = new MockStore();
    const n0 = s.scan('claude').items.filter((i) => i.category === 'mcp').length;
    const r = s.upsertMcpEntry('claude', 'global', 'brandnew', { command: 'npx', args: ['-y', 'p@1.0.0'] });
    const n1 = s.scan('claude').items.filter((i) => i.category === 'mcp').length;
    expect(n1).toBe(n0 + 1);
    s.restore({ ...r });
    const n2 = s.scan('claude').items.filter((i) => i.category === 'mcp').length;
    expect(n2).toBe(n0);
  });
});

describe('MockStore skill upsert', () => {
  it('skillUpsert adds a new skill item and undo removes it', () => {
    const s = new MockStore();
    const n0 = s.scan('claude').items.filter((i) => i.category === 'skill').length;
    const r = s.skillUpsert('claude', 'global', 'brand-skill', '---\nname: brand-skill\n---\n');
    expect(r.kind).toBe('skill-create');
    expect(s.scan('claude').items.filter((i) => i.category === 'skill').length).toBe(n0 + 1);
    s.restore({ ...r });
    expect(s.scan('claude').items.filter((i) => i.category === 'skill').length).toBe(n0);
  });
});
