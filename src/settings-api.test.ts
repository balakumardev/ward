import { vi, test, expect, beforeEach, afterEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { api } from './api';

// Mirror src/api.test.ts + src/plugins-api.test.ts: the api wrapper
// short-circuits when `window.__TAURI_INTERNALS__` is missing (the real "am I
// in a Tauri webview?" guard). Opt into "Tauri mode" by assigning the global
// before each test, then restore it after.
let originalInternals: unknown;
beforeEach(() => {
  invoke.mockReset();
  originalInternals = (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
});
afterEach(() => {
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = originalInternals;
});

// ── Plan 29: Settings mode ──

test('settingsCatalog invokes settings_catalog with no args', async () => {
  invoke.mockResolvedValue([]);
  const r = await api.settingsCatalog();
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(invoke.mock.calls[0][0]).toBe('settings_catalog');
  expect(invoke.mock.calls[0][1]).toBeUndefined();
  expect(r).toEqual([]);
});

test('settingsSet invokes settings_set with camelCase scope/key/targetFile/value', async () => {
  invoke.mockResolvedValue({ kind: 'setting-write', originalPath: '/Users/x/.claude/settings.json' });
  const r = await api.settingsSet('user', 'theme', 'settings.json', 'dark');
  expect(invoke).toHaveBeenCalledWith('settings_set', {
    scope: 'user', key: 'theme', targetFile: 'settings.json', value: 'dark',
  });
  expect(r.kind).toBe('setting-write');
});

test('settingsSet carries a whole-object value untouched (object-typed setting)', async () => {
  invoke.mockResolvedValue({ kind: 'setting-write', originalPath: '/Users/x/.claude/settings.json' });
  const perms = { defaultMode: 'acceptEdits', allow: ['Bash(git status)'] };
  await api.settingsSet('user', 'permissions', 'settings.json', perms);
  expect(invoke).toHaveBeenCalledWith('settings_set', {
    scope: 'user', key: 'permissions', targetFile: 'settings.json', value: perms,
  });
});

test('settingsUnset invokes settings_unset with scope/key/targetFile', async () => {
  invoke.mockResolvedValue({ kind: 'setting-write', originalPath: '/Users/x/.claude/settings.json' });
  const r = await api.settingsUnset('user', 'verbose', 'settings.json');
  expect(invoke).toHaveBeenCalledWith('settings_unset', {
    scope: 'user', key: 'verbose', targetFile: 'settings.json',
  });
  expect(r.kind).toBe('setting-write');
});

test('settingsSchemaDiff invokes settings_schema_diff with no args', async () => {
  invoke.mockResolvedValue({ inSchemaNotCatalog: ['someNewKey'], inCatalogNotSchema: [] });
  const r = await api.settingsSchemaDiff();
  expect(invoke).toHaveBeenCalledWith('settings_schema_diff', undefined);
  expect(r.inSchemaNotCatalog).toContain('someNewKey');
});

test('settingsEnvList invokes settings_env_list with no args', async () => {
  invoke.mockResolvedValue([{ name: 'ANTHROPIC_API_KEY', description: 'x', category: 'Authentication & Provider' }]);
  const r = await api.settingsEnvList();
  expect(invoke).toHaveBeenCalledWith('settings_env_list', undefined);
  expect(r[0].name).toBe('ANTHROPIC_API_KEY');
});
