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

describe('MockStore plugins', () => {
  it('pluginScan reports the seeded marketplaces + a mix of plugin states', () => {
    const s = new MockStore();
    const scan = s.pluginScan();
    expect(scan.cliAvailable).toBe(true);
    expect(s.pluginsCliAvailable()).toBe(true);
    expect(scan.marketplaces.length).toBeGreaterThanOrEqual(2);
    expect(scan.plugins.length).toBeGreaterThanOrEqual(3);
    // The mix the fixture guarantees: an enabled+installed, a disabled+installed,
    // a not-installed, and an uncatalogued (no componentCounts) entry.
    expect(scan.plugins.some((p) => p.installed && p.enabled)).toBe(true);
    expect(scan.plugins.some((p) => p.installed && !p.enabled)).toBe(true);
    expect(scan.plugins.some((p) => !p.installed)).toBe(true);
    expect(scan.plugins.some((p) => p.componentCounts === undefined)).toBe(true);
    expect(scan.plugins.some((p) => (p.componentCounts?.skills ?? 0) > 0)).toBe(true);
  });

  it('setPluginEnabled flips the entry and undo restores it', () => {
    const s = new MockStore();
    const key = 'code-formatter@claude-plugins-official';
    const before = s.pluginScan().plugins.find((p) => `${p.name}@${p.marketplace}` === key)!;
    expect(before.enabled).toBe(true);
    const r = s.setPluginEnabled(key, false);
    expect(r.kind).toBe('plugin-enable');
    expect(s.pluginScan().plugins.find((p) => `${p.name}@${p.marketplace}` === key)!.enabled).toBe(false);
    s.restore({ ...r });
    expect(s.pluginScan().plugins.find((p) => `${p.name}@${p.marketplace}` === key)!.enabled).toBe(true);
  });

  it('installPlugin marks a catalog-only entry installed + enabled', () => {
    const s = new MockStore();
    const target = s.pluginScan().plugins.find((p) => !p.installed)!;
    const scan = s.installPlugin(target.name, target.marketplace, 'user');
    const after = scan.plugins.find((p) => p.name === target.name && p.marketplace === target.marketplace)!;
    expect(after.installed).toBe(true);
    expect(after.enabled).toBe(true);
    expect(after.scope).toBe('user');
  });

  it('uninstallPlugin marks an installed entry not-installed + disabled', () => {
    const s = new MockStore();
    const scan = s.uninstallPlugin('code-formatter', 'user');
    const after = scan.plugins.find((p) => p.name === 'code-formatter')!;
    expect(after.installed).toBe(false);
    expect(after.enabled).toBe(false);
  });

  it('marketplaceAddPlugin adds a new marketplace derived from the src', () => {
    const s = new MockStore();
    const before = s.pluginScan().marketplaces.length;
    const scan = s.marketplaceAddPlugin('owner/new-market', 'user');
    expect(scan.marketplaces.length).toBe(before + 1);
    expect(scan.marketplaces.some((m) => m.name === 'new-market')).toBe(true);
    // Idempotent — adding the same src again does not duplicate.
    const again = s.marketplaceAddPlugin('owner/new-market', 'user');
    expect(again.marketplaces.length).toBe(before + 1);
  });

  it('marketplaceUpdatePlugins returns a fresh scan without mutating state', () => {
    const s = new MockStore();
    const before = s.pluginScan();
    const after = s.marketplaceUpdatePlugins();
    expect(after.plugins.length).toBe(before.plugins.length);
    expect(after.marketplaces.length).toBe(before.marketplaces.length);
  });
});

describe('MockStore settings', () => {
  it('catalog spans every editor branch + a managed and a claudeJson row', () => {
    const s = new MockStore();
    const rows = s.settingsCatalog();
    const types = new Set(rows.map((r) => r.def.valueType));
    // The four simple types + array + object are all present.
    for (const t of ['bool', 'enum', 'number', 'string', 'array', 'object']) {
      expect(types.has(t)).toBe(true);
    }
    // All five bespoke object editors are represented.
    const editors = new Set(rows.filter((r) => r.def.valueType === 'object').map((r) => r.def.editor));
    for (const e of ['permissions', 'hooks', 'env', 'sandbox', 'statusLine']) {
      expect(editors.has(e)).toBe(true);
    }
    // A read-only managed row and a ~/.claude.json-routed row.
    expect(rows.some((r) => r.def.managedOnly && r.sourceScope === 'managed')).toBe(true);
    expect(rows.some((r) => r.def.targetFile === 'claudeJson')).toBe(true);
    // A mix of set/unset.
    expect(rows.some((r) => r.isSet)).toBe(true);
    expect(rows.some((r) => !r.isSet)).toBe(true);
  });

  it('settingsSet marks the row set at user scope; undo restores it', () => {
    const s = new MockStore();
    const r = s.settingsSet('user', 'verbose', 'settings.json', true);
    expect(r.kind).toBe('setting-write');
    expect(r.originalPath).toBe('~/.claude/settings.json');
    const set = s.settingsCatalog().find((x) => x.def.key === 'verbose')!;
    expect(set.isSet).toBe(true);
    expect(set.effective).toBe(true);
    expect(set.sourceScope).toBe('user');
    s.restore({ ...r });
    expect(s.settingsCatalog().find((x) => x.def.key === 'verbose')!.isSet).toBe(false);
  });

  it('settingsUnset resets to default and routes claudeJson keys; undo restores', () => {
    const s = new MockStore();
    const r = s.settingsUnset('user', 'autoConnectIde', 'claudeJson');
    expect(r.kind).toBe('setting-write');
    expect(r.originalPath).toBe('~/.claude.json'); // target-file routing
    const row = s.settingsCatalog().find((x) => x.def.key === 'autoConnectIde')!;
    expect(row.isSet).toBe(false);
    expect(row.sourceScope).toBe('default');
    expect(row.effective).toBe(row.def.default); // false
    s.restore({ ...r });
    expect(s.settingsCatalog().find((x) => x.def.key === 'autoConnectIde')!.isSet).toBe(true);
  });

  it('schemaDiff + envList return the seeded samples', () => {
    const s = new MockStore();
    expect(s.settingsSchemaDiff().inSchemaNotCatalog).toContain('someNewKey');
    const env = s.settingsEnvList();
    expect(env.length).toBeGreaterThan(0);
    expect(env.every((e) => e.name && e.description && e.category)).toBe(true);
  });
});
