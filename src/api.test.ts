import { vi, test, expect, beforeEach, afterEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { api, TauriUnavailableError } from './api';

// The api wrapper short-circuits when `window.__TAURI_INTERNALS__` is
// missing (the real guard for "is this page running inside a Tauri
// webview?"). Tests that want to exercise the invoke path opt into
// "Tauri mode" by assigning the global before each test, then restore
// the original value after.
let originalInternals: unknown;
beforeEach(() => {
  invoke.mockReset();
  originalInternals = (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
});
afterEach(() => {
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = originalInternals;
});

test('scan calls invoke with harness arg', async () => {
  invoke.mockResolvedValue({ harnessId: 'claude', categories: [], scopes: [], items: [], capabilities: {} });
  const res = await api.scan('claude');
  expect(invoke).toHaveBeenCalledWith('scan', { harness: 'claude' });
  expect(res.harnessId).toBe('claude');
});

test('readFileContent passes path', async () => {
  invoke.mockResolvedValue('file body');
  const body = await api.readFileContent('/Users/x/.claude/CLAUDE.md');
  expect(invoke).toHaveBeenCalledWith('read_file_content', { path: '/Users/x/.claude/CLAUDE.md' });
  expect(body).toBe('file body');
});

test('moveItem passes harness, item, destScopeId', async () => {
  const item = { category: 'skill', scopeId: 'global', name: 'foo', path: '/p', movable: true, deletable: true, locked: false };
  invoke.mockResolvedValue({ kind: 'file', originalPath: '/p' });
  await api.moveItem('claude', item, 'global');
  expect(invoke).toHaveBeenCalledWith('move_item', { harness: 'claude', item, destScopeId: 'global' });
});

test('deleteItem passes harness + item', async () => {
  const item = { category: 'skill', scopeId: 'global', name: 'foo', path: '/p', movable: true, deletable: true, locked: false };
  invoke.mockResolvedValue({ kind: 'file', originalPath: '/p' });
  await api.deleteItem('claude', item);
  expect(invoke).toHaveBeenCalledWith('delete_item', { harness: 'claude', item });
});

test('restore passes harness + RestoreInfo', async () => {
  const info = { kind: 'file' as const, originalPath: '/p' };
  invoke.mockResolvedValue(undefined);
  await api.restore('claude', info);
  expect(invoke).toHaveBeenCalledWith('restore', { harness: 'claude', info });
});

test('saveFile passes path + content', async () => {
  invoke.mockResolvedValue(undefined);
  await api.saveFile('/p/foo.md', 'new body');
  expect(invoke).toHaveBeenCalledWith('save_file', { path: '/p/foo.md', content: 'new body' });
});

test('listDestinations passes harness + item', async () => {
  const item = { category: 'skill', scopeId: 'global', name: 'foo', path: '/p', movable: true, deletable: true, locked: false };
  invoke.mockResolvedValue([]);
  await api.listDestinations('claude', item);
  expect(invoke).toHaveBeenCalledWith('list_destinations', { harness: 'claude', item });
});

test('bulk passes op + items + optional dest', async () => {
  const items = [
    { category: 'skill', scopeId: 'global', name: 'a', path: '/a', movable: true, deletable: true, locked: false },
    { category: 'skill', scopeId: 'global', name: 'b', path: '/b', movable: true, deletable: true, locked: false },
  ];
  invoke.mockResolvedValue([]);
  await api.bulk('claude', items, 'move', 'repo-a');
  expect(invoke).toHaveBeenCalledWith('bulk', { harness: 'claude', items, op: 'move', destScopeId: 'repo-a' });
  await api.bulk('claude', items, 'delete');
  expect(invoke).toHaveBeenLastCalledWith('bulk', { harness: 'claude', items, op: 'delete', destScopeId: undefined });
});

test('bulkRestore passes harness + infos', async () => {
  invoke.mockResolvedValue(undefined);
  const infos = [{ kind: 'file' as const, originalPath: '/p' }];
  await api.bulkRestore('claude', infos);
  expect(invoke).toHaveBeenCalledWith('bulk_restore', { harness: 'claude', infos });
});

// ── Plan 04: MCP controls ──

test('mcpGetDisabled passes projectPath', async () => {
  invoke.mockResolvedValue([]);
  await api.mcpGetDisabled('/work/repo');
  expect(invoke).toHaveBeenCalledWith('mcp_get_disabled', { projectPath: '/work/repo' });
});

test('mcpSetDisabled passes projectPath + list', async () => {
  invoke.mockResolvedValue({ kind: 'mcp-disabled', originalPath: '/Users/x/.claude.json' });
  await api.mcpSetDisabled('/work/repo', ['github']);
  expect(invoke).toHaveBeenCalledWith('mcp_set_disabled', { projectPath: '/work/repo', list: ['github'] });
});

test('mcpGetPolicy passes no args', async () => {
  invoke.mockResolvedValue({ allowlist: [], denylist: [] });
  await api.mcpGetPolicy();
  // invoke is called with the command name + undefined args slot
  // (the wrapper always passes the second argument; Tauri ignores
  // `undefined` and treats it the same as omitting args).
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(invoke.mock.calls[0][0]).toBe('mcp_get_policy');
  expect(invoke.mock.calls[0][1]).toBeUndefined();
});

test('mcpSetPolicy passes policy', async () => {
  invoke.mockResolvedValue({ kind: 'mcp-policy', originalPath: '/Users/x/.claude/settings.json' });
  const policy = { allowlist: [{ serverName: 'github' }], denylist: [] };
  await api.mcpSetPolicy(policy);
  expect(invoke).toHaveBeenCalledWith('mcp_set_policy', { policy });
});

test('mcpCheckPolicy passes name + config + policy', async () => {
  invoke.mockResolvedValue('allowed');
  await api.mcpCheckPolicy('github', { command: 'gh' }, { allowlist: [], denylist: [] });
  expect(invoke).toHaveBeenCalledWith('mcp_check_policy', {
    serverName: 'github',
    serverConfig: { command: 'gh' },
    policy: { allowlist: [], denylist: [] },
  });
});

// ── Plan 06: Context Budget ──

test('contextBudget passes harness + scopeId', async () => {
  invoke.mockResolvedValue({
    systemLoaded: 18000, outputStyle: 0, systemDeferred: 7000,
    mcpSchemas: 3100, mcpToolNames: 120, claudemd: 100, claudeMdFiles: [],
    skillListing: 0, skillListingRaw: 0, skillBoilerplate: 0, agentListing: 0,
    alwaysLoadedItems: [], metadataItems: [], deferredItems: [],
    deferredTotal: 10100, autocompactBuffer: 13000,
    maxOutput: 32000, warningThreshold: 20000,
    measured: false, used: 18220, contextLimit: 200000,
  });
  const r = await api.contextBudget('claude', 'global');
  expect(invoke).toHaveBeenCalledWith('context_budget', { harness: 'claude', scopeId: 'global' });
  expect(r.used).toBe(18220);
  expect(r.contextLimit).toBe(200000);
  // MCP schemas are DEFERRED, not part of always-on `used`.
  expect(r.deferredTotal).toBe(10100);
  expect(r.mcpToolNames).toBe(120);
});

// ── Plan 08: Backup Center ──

test('backupStatus calls invoke with no args', async () => {
  invoke.mockResolvedValue({
    hasRepo: true, lastCommit: 'abc', lastCommitAt: null,
    schedulerInstalled: false, schedulerInterval: null, remoteUrl: null,
  });
  await api.backupStatus();
  expect(invoke).toHaveBeenCalledWith('backup_status', undefined);
});

test('backupRun passes scan + optional remoteUrl', async () => {
  const scan = { harnessId: 'claude', categories: [], scopes: [], items: [], capabilities: {} } as never;
  invoke.mockResolvedValue({ filesCopied: 3, bytesCopied: 100, skipped: [] });
  const r = await api.backupRun(scan, 'git@github.com:me/ward.git');
  expect(invoke).toHaveBeenCalledWith('backup_run', {
    scan, remoteUrl: 'git@github.com:me/ward.git',
  });
  expect(r.filesCopied).toBe(3);

  // Second call without remoteUrl should pass null.
  invoke.mockReset();
  invoke.mockResolvedValue({ filesCopied: 0, bytesCopied: 0, skipped: [] });
  await api.backupRun(scan);
  expect(invoke).toHaveBeenCalledWith('backup_run', { scan, remoteUrl: null });
});

test('backupSync passes no args', async () => {
  invoke.mockResolvedValue({ committed: true, sha: 'deadbeef', message: 'm', committedAt: null });
  await api.backupSync();
  expect(invoke).toHaveBeenCalledWith('backup_sync', undefined);
});

test('backupPush passes no args', async () => {
  invoke.mockResolvedValue({ pushed: false, reason: 'no remote configured', remoteUrl: null });
  await api.backupPush();
  expect(invoke).toHaveBeenCalledWith('backup_push', undefined);
});

test('backupSchedulerInstall passes intervalSeconds', async () => {
  invoke.mockResolvedValue(undefined);
  await api.backupSchedulerInstall(900);
  expect(invoke).toHaveBeenCalledWith('backup_scheduler_install', { intervalSeconds: 900 });
});

test('backupSchedulerRemove passes no args', async () => {
  invoke.mockResolvedValue(undefined);
  await api.backupSchedulerRemove();
  expect(invoke).toHaveBeenCalledWith('backup_scheduler_remove', undefined);
});

test('backupSetRemote passes url', async () => {
  invoke.mockResolvedValue(undefined);
  await api.backupSetRemote('git@github.com:me/ward.git');
  expect(invoke).toHaveBeenCalledWith('backup_set_remote', { url: 'git@github.com:me/ward.git' });
});

test('backupLog passes n', async () => {
  invoke.mockResolvedValue([
    { sha: 'deadbeef', subject: 'backup: ward', author: 'ward', committedAt: '2026-07-05T09:20:00Z' },
  ]);
  const r = await api.backupLog(20);
  expect(invoke).toHaveBeenCalledWith('backup_log', { n: 20 });
  expect(r[0].sha).toBe('deadbeef');
});

// ── Plan 15: menu-bar glance (usage / autostart / native status) ──

test('usageSnapshot passes harness', async () => {
  invoke.mockResolvedValue({
    harness: 'claude',
    block: {
      tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 },
      costUsd: 0, isActive: false,
    },
    week: {
      tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 },
      costUsd: 0, isActive: false,
    },
    source: 'local', available: false, generatedAt: '',
  });
  const r = await api.usageSnapshot('claude');
  expect(invoke).toHaveBeenCalledWith('usage_snapshot', { harness: 'claude' });
  expect(r.harness).toBe('claude');
});

test('usageSnapshotLive passes harness and returns the live source', async () => {
  invoke.mockResolvedValue({
    harness: 'claude',
    block: { tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 }, costUsd: 0, percent: 0.26, isActive: true },
    week: { tokens: { input: 0, output: 0, cacheCreation: 0, cacheRead: 0, total: 0 }, costUsd: 0, percent: 0.44, isActive: true },
    source: 'live', available: true, generatedAt: '',
  });
  const r = await api.usageSnapshotLive('claude');
  expect(invoke).toHaveBeenCalledWith('usage_snapshot_live', { harness: 'claude' });
  expect(r.source).toBe('live');
});

test('liveUsageEnabled calls invoke with no args', async () => {
  invoke.mockResolvedValue(false);
  const r = await api.liveUsageEnabled();
  expect(invoke).toHaveBeenCalledWith('live_usage_enabled', undefined);
  expect(r).toBe(false);
});

test('setLiveUsageEnabled passes enabled', async () => {
  invoke.mockResolvedValue(undefined);
  await api.setLiveUsageEnabled(true);
  expect(invoke).toHaveBeenCalledWith('set_live_usage_enabled', { enabled: true });
});

test('autostartStatus calls invoke with no args', async () => {
  invoke.mockResolvedValue(true);
  const r = await api.autostartStatus();
  expect(invoke).toHaveBeenCalledWith('autostart_status', undefined);
  expect(r).toBe(true);
});

test('autostartSet passes enabled', async () => {
  invoke.mockResolvedValue(undefined);
  await api.autostartSet(true);
  expect(invoke).toHaveBeenCalledWith('autostart_set', { enabled: true });
});

test('nativeUpdateStatus passes critical + optional lastScanAt', async () => {
  invoke.mockResolvedValue(undefined);
  await api.nativeUpdateStatus(2, '2026-07-05T00:00:00Z');
  expect(invoke).toHaveBeenCalledWith('native_update_status', {
    critical: 2, lastScanAt: '2026-07-05T00:00:00Z',
  });

  // Second call without lastScanAt should pass undefined.
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  await api.nativeUpdateStatus(0);
  expect(invoke).toHaveBeenCalledWith('native_update_status', {
    critical: 0, lastScanAt: undefined,
  });
});

// ── Plan 18: MCP marketplace (upsert) ──

test('mcpUpsertEntry invokes mcp_upsert_entry with camelCase args', async () => {
  invoke.mockResolvedValue({ kind: 'mcp-upsert', originalPath: '/x' });
  await api.mcpUpsertEntry('claude', 'global', 'srv', { command: 'npx', args: ['-y', 'p@1.0.0'] }, '/Users/x/.claude.json');
  expect(invoke).toHaveBeenCalledWith('mcp_upsert_entry', {
    harness: 'claude', scopeId: 'global', name: 'srv',
    config: { command: 'npx', args: ['-y', 'p@1.0.0'] }, targetPath: '/Users/x/.claude.json',
  });
});

// ── Plan 19: creatable skills (skill_upsert) ──

test('skillUpsert invokes skill_upsert with camelCase args', async () => {
  invoke.mockResolvedValue({ kind: 'skill-create', originalPath: '/x' });
  await api.skillUpsert('claude', 'global', 'my-skill', '---\nname: my-skill\n---\n');
  expect(invoke).toHaveBeenCalledWith('skill_upsert',
    { harness: 'claude', scopeId: 'global', name: 'my-skill', content: '---\nname: my-skill\n---\n' });
});

// ── Tauri runtime detection ────────────────────────────────────────────

test('invoke rejects with TauriUnavailableError when not running inside a Tauri webview', async () => {
  // Remove the guard installed in beforeEach to simulate the bare
  // Vite browser preview (no Tauri runtime).
  delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  invoke.mockClear();

  await expect(api.scan('claude')).rejects.toBeInstanceOf(TauriUnavailableError);
  // `invoke` must NOT have been called — the wrapper short-circuits.
  expect(invoke).not.toHaveBeenCalled();
});
