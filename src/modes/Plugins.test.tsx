import { test, expect, vi } from 'vitest';
import { render, fireEvent, waitFor } from '@solidjs/testing-library';
import { Plugins } from './Plugins';
import type { PluginsApi } from './Plugins';
import type { PluginScan, ScanResult } from '../api';

/** A PluginScan with a deliberate mix of states — one installed+enabled
 *  (fully catalogued) and one not-installed (discoverable) — plus two
 *  marketplaces, so every Discover branch is exercisable. */
function makeScan(over: Partial<PluginScan> = {}): PluginScan {
  return {
    cliAvailable: true,
    marketplaces: [
      { name: 'claude-plugins-official', source: { source: 'github', repo: 'anthropics/x' }, installLocation: '~/.claude/plugins/marketplaces/claude-plugins-official' },
      { name: 'side-marketplace', source: { source: 'github', repo: 'community/y' }, installLocation: '~/.claude/plugins/marketplaces/side-marketplace' },
    ],
    plugins: [
      {
        kind: 'plugin', name: 'code-formatter', marketplace: 'claude-plugins-official',
        displayName: 'Code Formatter', description: 'Opinionated multi-language formatter.',
        version: '2.1.0', source: { source: 'github', repo: 'anthropics/x' }, author: 'Anthropic',
        category: 'Productivity', tags: ['formatting'], installed: true, enabled: true, scope: 'user',
        uniqueInstalls: 682, alwaysOnTokens: 1005, onInvokeTokens: 15353,
        componentCounts: { commands: 1, agents: 0, skills: 2, hooks: 0, mcpServers: 1, lspServers: 0 },
      },
      {
        kind: 'plugin', name: 'security-scanner', marketplace: 'claude-plugins-official',
        displayName: 'Security Scanner', description: 'Scan configs and MCP servers for risky patterns.',
        version: '3.0.1', source: { source: 'github', repo: 'anthropics/x' }, author: 'Anthropic',
        category: 'Security', tags: ['security'], installed: false, enabled: false,
        uniqueInstalls: 1290, alwaysOnTokens: 1420, onInvokeTokens: 20110,
        componentCounts: { commands: 2, agents: 1, skills: 1, hooks: 1, mcpServers: 1, lspServers: 0 },
      },
    ],
    ...over,
  };
}

/** The api surface Plugins needs. `scan` re-reads the on-disk state; install /
 *  marketplaceAdd return a fresh PluginScan (the CLI-backed commands re-scan). */
function makeApi(over: Partial<PluginsApi> = {}, scan = makeScan()): PluginsApi {
  return {
    scan: vi.fn(async () => scan),
    install: vi.fn(async () => makeScan()),
    marketplaceAdd: vi.fn(async () => makeScan()),
    cliAvailable: vi.fn(async () => scan.cliAvailable),
    ...over,
  };
}

const CAPS = {
  contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true,
  sessions: true, effective: true, backup: true, mcpEditable: true, skillCreatable: true,
  pluginsManageable: true,
};

function makeHostScan(pluginsManageable: boolean): ScanResult {
  return {
    harnessId: pluginsManageable ? 'claude' : 'codex',
    categories: [],
    scopes: [{ id: 'global', kind: 'global', label: 'Global', root: '/Users/x/.claude' }],
    items: [],
    capabilities: { ...CAPS, pluginsManageable },
  };
}

// ── Task 9: mode + capability gate ──────────────────────────────────────────

test('renders plugins mode when pluginsManageable is true', async () => {
  const { findByTestId } = render(() => <Plugins scan={makeHostScan(true)} api={makeApi()} />);
  await findByTestId('plugins-mode');
});

test('shows unsupported panel when pluginsManageable is false', async () => {
  const { getByTestId, queryByTestId } = render(() => <Plugins scan={makeHostScan(false)} api={makeApi()} />);
  const panel = getByTestId('plugins-unsupported');
  expect(panel.textContent).toMatch(/not supported|not applicable|Plugins/i);
  expect(queryByTestId('plugins-mode')).toBeNull();
});

// ── Task 10: Discover tab ────────────────────────────────────────────────────

test('discover lists available plugins with source badge', async () => {
  const { findByTestId, findAllByTestId } = render(() => <Plugins scan={makeHostScan(true)} api={makeApi()} />);
  await findByTestId('plugins-discover');
  const cards = await findAllByTestId('plugin-card');
  expect(cards.length).toBe(2);
  // Every card carries a source (marketplace) badge.
  const badges = await findAllByTestId('plugin-source');
  expect(badges.length).toBe(2);
  expect(badges.map((b) => b.textContent)).toEqual(
    expect.arrayContaining(['claude-plugins-official']),
  );
  // Token cost + component counts (the Ward differentiator) surface on the card.
  const text = cards.map((c) => c.textContent).join(' ');
  expect(text).toContain('1,005'); // alwaysOnTokens, locale-formatted
  expect(text).toContain('MCP server'); // componentCounts summary
});

test('install opens in-app confirm modal then calls pluginsInstall', async () => {
  const api = makeApi();
  const { findByTestId, getByTestId } = render(() => <Plugins scan={makeHostScan(true)} api={api} />);
  await findByTestId('plugins-discover');

  // Only the not-installed plugin (security-scanner) offers an Install button.
  const installBtn = await findByTestId('plugin-install');
  fireEvent.click(installBtn);

  // A real in-app modal (WKWebView's confirm() is a no-op) — role=dialog, aria-modal.
  const dialog = await findByTestId('plugin-install-confirm');
  expect(dialog.getAttribute('role')).toBe('dialog');
  expect(dialog.getAttribute('aria-modal')).toBe('true');
  // pluginsInstall is not called until the user confirms.
  expect(api.install).not.toHaveBeenCalled();

  fireEvent.click(getByTestId('plugin-confirm-ok'));
  await waitFor(() => expect(api.install).toHaveBeenCalledTimes(1));
  expect((api.install as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
    'security-scanner', 'claude-plugins-official', 'user',
  ]);

  // The reload toast tells the user Ward can't reload plugins itself.
  const toast = await findByTestId('plugin-reload-toast');
  expect(toast.textContent).toMatch(/restart Claude Code|reload-plugins/i);
});

test('cli-absent banner shows when cliAvailable is false and disables Install', async () => {
  const api = makeApi({}, makeScan({ cliAvailable: false }));
  const { findByTestId } = render(() => <Plugins scan={makeHostScan(true)} api={api} />);
  const banner = await findByTestId('plugin-cli-banner');
  expect(banner.textContent).toMatch(/CLI|PATH/i);
  // Install needs the CLI, so its button is disabled while the CLI is absent.
  const installBtn = (await findByTestId('plugin-install')) as HTMLButtonElement;
  expect(installBtn.disabled).toBe(true);
});
