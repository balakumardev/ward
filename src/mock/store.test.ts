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

describe('MockStore marketplace', () => {
  it('marketplaceSearch filters MCP servers by substring', () => {
    const s = new MockStore();
    expect(s.marketplaceSearch('mcp', '').entries.length).toBeGreaterThanOrEqual(3);
    const hits = s.marketplaceSearch('mcp', 'pytools').entries;
    expect(hits.length).toBe(1);
    expect(hits[0].name).toContain('pytools');
    // An unknown kind → empty page (not an error).
    expect(s.marketplaceSearch('other', '').entries.length).toBe(0);
  });

  it('marketplaceSearch returns curated skills filtered by substring', () => {
    const s = new MockStore();
    expect(s.marketplaceSearch('skill', '').entries.length).toBeGreaterThanOrEqual(3);
    const hits = s.marketplaceSearch('skill', 'debug').entries;
    expect(hits.length).toBe(1);
    expect(hits[0].name).toContain('debugging');
    expect(hits[0].kind).toBe('skill');
    expect(hits[0].skillPath).toBeTruthy();
  });

  it('marketplacePreviewSkill returns the SKILL.md body + frontmatter meta', () => {
    const s = new MockStore();
    const entry = s.marketplaceSearch('skill', 'brainstorming').entries[0];
    const preview = s.marketplacePreviewSkill(entry);
    expect(preview.name).toBe('brainstorming');
    expect(preview.description).toContain('Explore');
    expect(preview.body).toContain('---');
    expect(preview.body.toLowerCase()).toContain('brainstorming');
  });

  it('marketplaceInstall of a skill to Claude global adds a skill item', () => {
    const s = new MockStore();
    const entry = s.marketplaceSearch('skill', 'brainstorming').entries[0];
    const before = s.scan('claude').items.filter((i) => i.category === 'skill').length;
    const results = s.marketplaceInstall(entry, 0, [{ harness: 'claude', scopeId: 'global' }], {});
    expect(results.length).toBe(1);
    expect(results[0].ok).toBe(true);
    const items = s.scan('claude').items.filter((i) => i.category === 'skill');
    expect(items.length).toBe(before + 1);
    expect(items.some((i) => i.name === 'brainstorming')).toBe(true);
  });

  it('marketplaceInstall of a skill is create-only per target', () => {
    const s = new MockStore();
    const entry = s.marketplaceSearch('skill', 'brainstorming').entries[0];
    s.marketplaceInstall(entry, 0, [{ harness: 'claude', scopeId: 'global' }], {});
    // Installing the same skill to the same target again fails (already exists),
    // without aborting or double-adding.
    const again = s.marketplaceInstall(entry, 0, [{ harness: 'claude', scopeId: 'global' }], {});
    expect(again[0].ok).toBe(false);
    expect(again[0].error).toContain('already exists');
    expect(s.scan('claude').items.filter((i) => i.category === 'skill' && i.name === 'brainstorming').length).toBe(1);
  });

  it('marketplaceBuildConfig pins the version and omits secret env values', () => {
    const s = new MockStore();
    const entry = s.marketplaceSearch('mcp', 'notes').entries[0];
    const built = s.marketplaceBuildConfig(entry, 0, { NOTES_REGION: 'us-east-1', NOTES_API_KEY: 'leak' });
    expect(built.config.command).toBe('npx');
    expect(built.config.args).toEqual(['-y', '@acme/notes-mcp@2.1.0']);
    expect(built.config.env).toEqual({ NOTES_REGION: 'us-east-1' });
    expect(JSON.stringify(built.config)).not.toContain('leak'); // secret never written
  });

  it('marketplaceInstall to Claude global adds an MCP item and reports ok', () => {
    const s = new MockStore();
    const entry = s.marketplaceSearch('mcp', 'notes').entries[0];
    const before = s.scan('claude').items.filter((i) => i.category === 'mcp').length;
    const results = s.marketplaceInstall(entry, 0, [{ harness: 'claude', scopeId: 'global' }], {});
    expect(results.length).toBe(1);
    expect(results[0].ok).toBe(true);
    const items = s.scan('claude').items.filter((i) => i.category === 'mcp');
    expect(items.length).toBe(before + 1);
    expect(items.some((i) => i.name === 'notes')).toBe(true);
  });

  it('marketplaceInstall does not abort the batch on a build failure', () => {
    const s = new MockStore();
    const entry = s.marketplaceSearch('mcp', 'notes').entries[0];
    entry.packages[0].version = 'latest'; // force an unpinned build failure
    const results = s.marketplaceInstall(entry, 0, [
      { harness: 'claude', scopeId: 'global' },
      { harness: 'codex', scopeId: 'global' },
    ], {});
    expect(results.length).toBe(2); // every target attempted
    expect(results.every((r) => !r.ok)).toBe(true);
    expect(results[0].error).toContain('unpinned');
  });
});
