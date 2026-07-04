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
