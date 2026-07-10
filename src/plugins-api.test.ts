import { vi, test, expect, beforeEach, afterEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { api } from './api';

// Mirror src/api.test.ts: the api wrapper short-circuits when
// `window.__TAURI_INTERNALS__` is missing (the real "am I in a Tauri
// webview?" guard). Opt into "Tauri mode" by assigning the global before
// each test, then restore it after.
let originalInternals: unknown;
beforeEach(() => {
  invoke.mockReset();
  originalInternals = (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
});
afterEach(() => {
  (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = originalInternals;
});

// ── Plan 28: Plugins mode ──

test('pluginsScan invokes plugins_scan with no args', async () => {
  invoke.mockResolvedValue({ marketplaces: [], plugins: [], cliAvailable: true });
  const r = await api.pluginsScan();
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(invoke.mock.calls[0][0]).toBe('plugins_scan');
  expect(invoke.mock.calls[0][1]).toBeUndefined();
  expect(r.cliAvailable).toBe(true);
});

test('pluginsCliAvailable invokes plugins_cli_available with no args', async () => {
  invoke.mockResolvedValue(true);
  const r = await api.pluginsCliAvailable();
  expect(invoke).toHaveBeenCalledWith('plugins_cli_available', undefined);
  expect(r).toBe(true);
});

test('pluginsSetEnabled invokes plugins_set_enabled with camelCase args', async () => {
  invoke.mockResolvedValue({ kind: 'plugin-enable', originalPath: '/Users/x/.claude/settings.json' });
  const r = await api.pluginsSetEnabled('x@m', false);
  expect(invoke).toHaveBeenCalledWith('plugins_set_enabled', { pluginKey: 'x@m', enabled: false });
  expect(r.kind).toBe('plugin-enable');
});

test('pluginsInstall passes plugin + marketplace + scope', async () => {
  invoke.mockResolvedValue({ marketplaces: [], plugins: [], cliAvailable: true });
  await api.pluginsInstall('code-formatter', 'claude-plugins-official', 'user');
  expect(invoke).toHaveBeenCalledWith('plugins_install', {
    plugin: 'code-formatter', marketplace: 'claude-plugins-official', scope: 'user',
  });
});

test('pluginsUninstall passes plugin + scope', async () => {
  invoke.mockResolvedValue({ marketplaces: [], plugins: [], cliAvailable: true });
  await api.pluginsUninstall('code-formatter', 'user');
  expect(invoke).toHaveBeenCalledWith('plugins_uninstall', { plugin: 'code-formatter', scope: 'user' });
});

test('pluginsMarketplaceAdd passes src + scope', async () => {
  invoke.mockResolvedValue({ marketplaces: [], plugins: [], cliAvailable: true });
  await api.pluginsMarketplaceAdd('owner/repo', 'user');
  expect(invoke).toHaveBeenCalledWith('plugins_marketplace_add', { src: 'owner/repo', scope: 'user' });
});

test('pluginsMarketplaceUpdate passes a named marketplace, or undefined for all', async () => {
  invoke.mockResolvedValue({ marketplaces: [], plugins: [], cliAvailable: true });
  await api.pluginsMarketplaceUpdate('claude-plugins-official');
  expect(invoke).toHaveBeenCalledWith('plugins_marketplace_update', { name: 'claude-plugins-official' });

  // No argument → update every known marketplace (name: undefined).
  invoke.mockReset();
  invoke.mockResolvedValue({ marketplaces: [], plugins: [], cliAvailable: true });
  await api.pluginsMarketplaceUpdate();
  expect(invoke).toHaveBeenCalledWith('plugins_marketplace_update', { name: undefined });
});
