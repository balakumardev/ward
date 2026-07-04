import { vi, test, expect, beforeEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { api } from './api';

beforeEach(() => invoke.mockReset());

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
  // invoke is called with just the command name; no payload object.
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(invoke.mock.calls[0][0]).toBe('mcp_get_policy');
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
    systemLoaded: 18000, systemDeferred: 7000,
    mcpSchemas: 0, claudemd: 100, claudeMdFiles: [],
    alwaysLoadedItems: [], autocompactBuffer: 13000,
    maxOutput: 32000, warningThreshold: 20000,
    measured: false, used: 18100, contextLimit: 200000,
  });
  const r = await api.contextBudget('claude', 'global');
  expect(invoke).toHaveBeenCalledWith('context_budget', { harness: 'claude', scopeId: 'global' });
  expect(r.used).toBe(18100);
  expect(r.contextLimit).toBe(200000);
});

// ── Plan 08: Backup Center ──

test('backupStatus calls invoke with no args', async () => {
  invoke.mockResolvedValue({
    hasRepo: true, lastCommit: 'abc', lastCommitAt: null,
    schedulerInstalled: false, schedulerInterval: null, remoteUrl: null,
  });
  await api.backupStatus();
  expect(invoke).toHaveBeenCalledWith('backup_status');
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
  expect(invoke).toHaveBeenCalledWith('backup_sync');
});

test('backupPush passes no args', async () => {
  invoke.mockResolvedValue({ pushed: false, reason: 'no remote configured', remoteUrl: null });
  await api.backupPush();
  expect(invoke).toHaveBeenCalledWith('backup_push');
});

test('backupSchedulerInstall passes intervalSeconds', async () => {
  invoke.mockResolvedValue(undefined);
  await api.backupSchedulerInstall(900);
  expect(invoke).toHaveBeenCalledWith('backup_scheduler_install', { intervalSeconds: 900 });
});

test('backupSchedulerRemove passes no args', async () => {
  invoke.mockResolvedValue(undefined);
  await api.backupSchedulerRemove();
  expect(invoke).toHaveBeenCalledWith('backup_scheduler_remove');
});

test('backupSetRemote passes url', async () => {
  invoke.mockResolvedValue(undefined);
  await api.backupSetRemote('git@github.com:me/ward.git');
  expect(invoke).toHaveBeenCalledWith('backup_set_remote', { url: 'git@github.com:me/ward.git' });
});
