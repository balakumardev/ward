import { test, expect, vi } from 'vitest';
import { render, fireEvent, waitFor } from '@solidjs/testing-library';
import { Settings } from './Settings';
import type { SettingsApi } from './Settings';
import type { RestoreInfo, ScanResult, SettingRow } from '../api';

/** A representative catalog spanning every editor branch the core list handles:
 *  a bool (UNSET → falls through to its default), an enum (set/user), a number
 *  (set/project), a string (set/user), an array + an object (both "Edit…" only
 *  for now — array/object editors land in a later task), a managed-only enum
 *  (read-only, source `managed`), and a claudeJson-routed bool. One row per
 *  category so category filtering collapses to a single, assertable row.
 *  Mirrors the Rust `SettingRow` wire shape (`{def, effective?, sourceScope?,
 *  isSet}`). */
function makeCatalog(): SettingRow[] {
  const scopes = ['user', 'project', 'local'];
  return [
    {
      def: {
        key: 'verbose', label: 'Verbose output',
        description: 'Show full command output and tool detail.',
        category: 'Output', valueType: 'bool', default: false,
        enumValues: [], targetFile: 'settings.json', scopes, managedOnly: false,
      },
      isSet: false,
    },
    {
      def: {
        key: 'theme', label: 'Color theme',
        description: 'The color theme for the terminal UI.',
        category: 'Appearance', valueType: 'enum', default: 'dark',
        enumValues: ['dark', 'light', 'dark-daltonized'],
        targetFile: 'settings.json', scopes, managedOnly: false,
      },
      effective: 'dark', sourceScope: 'user', isSet: true,
    },
    {
      def: {
        key: 'cleanupPeriodDays', label: 'Chat retention (days)',
        description: 'How many days to retain chat transcripts before cleanup.',
        category: 'Privacy', valueType: 'number', default: 30,
        enumValues: [], targetFile: 'settings.json', scopes, managedOnly: false,
      },
      effective: 45, sourceScope: 'project', isSet: true,
    },
    {
      def: {
        key: 'model', label: 'Default model',
        description: 'The model alias Claude Code uses for the main loop.',
        category: 'Model', valueType: 'string',
        enumValues: [], targetFile: 'settings.json', scopes, managedOnly: false,
      },
      effective: 'claude-opus-4-8', sourceScope: 'user', isSet: true,
    },
    {
      def: {
        key: 'enabledMcpjsonServers', label: 'Enabled .mcp.json servers',
        description: 'Names of project `.mcp.json` servers approved to run.',
        category: 'MCP', valueType: 'array', default: [],
        enumValues: [], targetFile: 'settings.json', scopes, managedOnly: false,
      },
      effective: ['context7', 'postman'], sourceScope: 'project', isSet: true,
    },
    {
      def: {
        key: 'permissions', label: 'Permissions',
        description: 'Tool permission rules (allow / ask / deny) and default mode.',
        category: 'Permissions', valueType: 'object', editor: 'permissions',
        enumValues: [], targetFile: 'settings.json', scopes, managedOnly: false,
      },
      effective: { defaultMode: 'acceptEdits', allow: ['Bash(git status)'] },
      sourceScope: 'user', isSet: true,
    },
    {
      def: {
        key: 'forceLoginMethod', label: 'Force login method',
        description: 'Restrict sign-in to a single method (enterprise-managed only).',
        category: 'Enterprise', valueType: 'enum',
        enumValues: ['claudeai', 'console'],
        targetFile: 'settings.json', scopes: ['managed'], managedOnly: true,
      },
      effective: 'claudeai', sourceScope: 'managed', isSet: true,
    },
    {
      def: {
        key: 'autoConnectIde', label: 'Auto-connect IDE',
        description: 'Connect to a running IDE extension on startup.',
        category: 'IDE', valueType: 'bool', default: false,
        enumValues: [], targetFile: 'claudeJson', scopes: ['user'], managedOnly: false,
      },
      effective: true, sourceScope: 'user', isSet: true,
    },
  ];
}

/** The api surface Settings needs. `catalog` re-reads the joined catalog+state;
 *  set / unset are surgical single-key writes that return a `setting-write`
 *  RestoreInfo for Undo; restore reverses one via the shared engine. */
function makeApi(over: Partial<SettingsApi> = {}, catalog = makeCatalog()): SettingsApi {
  return {
    catalog: vi.fn(async () => catalog),
    set: vi.fn(async (): Promise<RestoreInfo> => ({ kind: 'setting-write', originalPath: '/Users/x/.claude/settings.json' })),
    unset: vi.fn(async (): Promise<RestoreInfo> => ({ kind: 'setting-write', originalPath: '/Users/x/.claude/settings.json' })),
    restore: vi.fn(async () => {}),
    ...over,
  };
}

const CAPS = {
  contextBudget: true, mcpControls: true, mcpPolicy: true, mcpSecurity: true,
  sessions: true, effective: true, backup: true, mcpEditable: true, skillCreatable: true,
  pluginsManageable: true, settingsEditable: true,
};

function makeHostScan(settingsEditable: boolean): ScanResult {
  return {
    harnessId: settingsEditable ? 'claude' : 'codex',
    categories: [],
    scopes: [{ id: 'global', kind: 'global', label: 'Global', root: '/Users/x/.claude' }],
    items: [],
    capabilities: { ...CAPS, settingsEditable },
  };
}

/** Locate a single row by its stable `data-key` (the def.key). */
function rowByKey(container: HTMLElement, key: string): HTMLElement {
  const el = container.querySelector(`[data-testid="setting-row"][data-key="${key}"]`);
  if (!el) throw new Error(`no setting-row for key "${key}"`);
  return el as HTMLElement;
}

// ── Task 10: mode + capability gate ──────────────────────────────────────────

test('renders settings mode when settingsEditable is true', async () => {
  const { findByTestId } = render(() => <Settings scan={makeHostScan(true)} api={makeApi()} />);
  await findByTestId('settings-mode');
});

test('shows unsupported panel when settingsEditable is false (codex)', async () => {
  const { getByTestId, queryByTestId } = render(() => <Settings scan={makeHostScan(false)} api={makeApi()} />);
  const panel = getByTestId('settings-unsupported');
  expect(panel.textContent).toMatch(/not supported|not editable|Settings/i);
  expect(queryByTestId('settings-mode')).toBeNull();
});

// ── Task 11: core list ───────────────────────────────────────────────────────

test('list renders rows with label, effective value, and source chip', async () => {
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={makeApi()} />);
  await findByTestId('settings-mode');

  const themeRow = await waitFor(() => rowByKey(container, 'theme'));
  expect(themeRow.textContent).toContain('Color theme');
  // Effective value surfaces on the row.
  const val = themeRow.querySelector('[data-testid="setting-value"]');
  expect(val?.textContent).toContain('dark');
  // Source-scope chip shows where the effective value came from.
  const chip = themeRow.querySelector('[data-testid="setting-source"]');
  expect(chip?.textContent).toMatch(/user/i);

  // An unset row's chip reads `default` (nothing set it in the scope chain).
  const verboseChip = rowByKey(container, 'verbose').querySelector('[data-testid="setting-source"]');
  expect(verboseChip?.textContent).toMatch(/default/i);
});

test('toggling a bool calls set with (user, key, targetFile, newValue)', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const row = await waitFor(() => rowByKey(container, 'verbose'));
  const toggle = row.querySelector('[data-testid="setting-toggle"]') as HTMLButtonElement;
  // Unset bool falls through to its default (false).
  expect(toggle.getAttribute('aria-checked')).toBe('false');
  fireEvent.click(toggle);

  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  // Scope is fixed 'user'; targetFile comes from the def, value is the flipped bool.
  expect((api.set as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
    'user', 'verbose', 'settings.json', true,
  ]);

  // A toast surfaces with an Undo (bound to the returned RestoreInfo) + a restart note.
  const toast = await findByTestId('settings-toast');
  expect(toast.textContent).toMatch(/restart/i);
  expect(toast.textContent).toMatch(/Claude Code/i);
  await findByTestId('settings-undo');
});

test('changing an enum calls set with the selected value', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const row = await waitFor(() => rowByKey(container, 'theme'));
  const select = row.querySelector('[data-testid="setting-enum"]') as HTMLSelectElement;
  fireEvent.change(select, { target: { value: 'light' } });

  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  expect((api.set as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
    'user', 'theme', 'settings.json', 'light',
  ]);
});

test('reset calls unset(user, key, targetFile); hidden for unset rows', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const themeRow = await waitFor(() => rowByKey(container, 'theme'));
  const reset = themeRow.querySelector('[data-testid="setting-reset"]') as HTMLButtonElement;
  expect(reset).toBeTruthy();
  // An unset row has nothing to reset.
  expect(rowByKey(container, 'verbose').querySelector('[data-testid="setting-reset"]')).toBeNull();

  fireEvent.click(reset);
  await waitFor(() => expect(api.unset).toHaveBeenCalledTimes(1));
  expect((api.unset as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
    'user', 'theme', 'settings.json',
  ]);
});

test('managed row editor is read-only with an indicator and no reset', async () => {
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={makeApi()} />);
  await findByTestId('settings-mode');

  const row = await waitFor(() => rowByKey(container, 'forceLoginMethod'));
  // A managed indicator explains why the editor is locked.
  expect(row.querySelector('[data-testid="setting-managed"]')).toBeTruthy();
  // Its enum editor is disabled — a user can't override managed settings.
  const select = row.querySelector('[data-testid="setting-enum"]') as HTMLSelectElement;
  expect(select.disabled).toBe(true);
  // Reset is meaningless (user scope can't override managed) → not shown.
  expect(row.querySelector('[data-testid="setting-reset"]')).toBeNull();
});

test('array/object rows show an inert Edit… button (no crash, no editor yet)', async () => {
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={makeApi()} />);
  await findByTestId('settings-mode');

  const arrRow = await waitFor(() => rowByKey(container, 'enabledMcpjsonServers'));
  const arrEdit = arrRow.querySelector('[data-testid="setting-edit"]') as HTMLButtonElement;
  expect(arrEdit).toBeTruthy();
  expect(arrEdit.disabled).toBe(true);

  const objEdit = rowByKey(container, 'permissions').querySelector('[data-testid="setting-edit"]') as HTMLButtonElement;
  expect(objEdit).toBeTruthy();
  expect(objEdit.disabled).toBe(true);
});

test('category filter and search narrow the list', async () => {
  const { findByTestId, getByTestId, getAllByTestId } = render(() => (
    <Settings scan={makeHostScan(true)} api={makeApi()} />
  ));
  await findByTestId('settings-mode');
  await waitFor(() => expect(getAllByTestId('setting-row').length).toBeGreaterThan(1));
  const allCount = getAllByTestId('setting-row').length;

  // Clicking the Appearance category filters to its single row (theme).
  const cats = getAllByTestId('settings-cat');
  const appearance = cats.find((c) => /Appearance/i.test(c.textContent || ''))!;
  fireEvent.click(appearance);
  await waitFor(() => {
    const rows = getAllByTestId('setting-row');
    expect(rows.length).toBe(1);
    expect(rows[0].getAttribute('data-key')).toBe('theme');
  });

  // Back to "All" restores the full list.
  const all = cats.find((c) => /^\s*All/i.test(c.textContent || ''))!;
  fireEvent.click(all);
  await waitFor(() => expect(getAllByTestId('setting-row').length).toBe(allCount));

  // A text search narrows by label / key / description.
  const search = getByTestId('settings-search') as HTMLInputElement;
  fireEvent.input(search, { target: { value: 'retention' } });
  await waitFor(() => {
    const rows = getAllByTestId('setting-row');
    expect(rows.length).toBe(1);
    expect(rows[0].getAttribute('data-key')).toBe('cleanupPeriodDays');
  });
});
