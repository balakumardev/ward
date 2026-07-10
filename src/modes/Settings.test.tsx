import { test, expect, vi } from 'vitest';
import { render, fireEvent, waitFor } from '@solidjs/testing-library';
import { Settings } from './Settings';
import type { SettingsApi } from './Settings';
import type { EnvVarDef, RestoreInfo, SchemaDiff, ScanResult, SettingRow } from '../api';

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
    {
      def: {
        key: 'env', label: 'Environment variables',
        description: 'Environment variables applied to every Claude Code session.',
        category: 'Environment', valueType: 'object', editor: 'env',
        enumValues: [], targetFile: 'settings.json', scopes, managedOnly: false,
      },
      effective: {}, sourceScope: 'user', isSet: true,
    },
    {
      def: {
        key: 'worktree', label: 'Worktree settings',
        description: 'Advanced worktree configuration edited as raw JSON.',
        category: 'Advanced', valueType: 'object', editor: 'json',
        enumValues: [], targetFile: 'settings.json', scopes, managedOnly: false,
      },
      effective: {}, sourceScope: 'user', isSet: true,
    },
    // object (editor: hooks) — stays inert (Task 14).
    {
      def: {
        key: 'hooks', label: 'Hooks',
        description: 'Shell commands the harness runs on lifecycle events.',
        category: 'Hooks', valueType: 'object', editor: 'hooks',
        enumValues: [], targetFile: 'settings.json', scopes, managedOnly: false,
      },
      isSet: false,
    },
    // object (editor: statusLine) — UNSET; a unique category so it doesn't
    // collide with `theme`'s Appearance in the category-filter test.
    {
      def: {
        key: 'statusLine', label: 'Status line',
        description: 'A custom command whose output replaces the default status line.',
        category: 'Status Line', valueType: 'object', editor: 'statusLine',
        enumValues: [], targetFile: 'settings.json', scopes, managedOnly: false,
      },
      isSet: false,
    },
    // object (editor: sandbox) — UNSET.
    {
      def: {
        key: 'sandbox', label: 'Sandbox',
        description: 'Filesystem and network allow / deny rules for the command sandbox.',
        category: 'Sandbox', valueType: 'object', editor: 'sandbox',
        enumValues: [], targetFile: 'settings.json', scopes, managedOnly: false,
      },
      isSet: false,
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
    schemaDiff: vi.fn(async (): Promise<SchemaDiff> => ({ inSchemaNotCatalog: [], inCatalogNotSchema: [] })),
    envList: vi.fn(async (): Promise<EnvVarDef[]> => []),
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

test('array/env/json + bespoke permissions/statusLine/sandbox/hooks rows are all editable', async () => {
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={makeApi()} />);
  await findByTestId('settings-mode');

  // Array/env/json editors ship in Task 12 → their Edit… button is enabled.
  const arrEdit = (await waitFor(() => rowByKey(container, 'enabledMcpjsonServers')))
    .querySelector('[data-testid="setting-edit"]') as HTMLButtonElement;
  expect(arrEdit).toBeTruthy();
  expect(arrEdit.disabled).toBe(false);

  const envEdit = rowByKey(container, 'env').querySelector('[data-testid="setting-edit"]') as HTMLButtonElement;
  expect(envEdit.disabled).toBe(false);

  const jsonEdit = rowByKey(container, 'worktree').querySelector('[data-testid="setting-edit"]') as HTMLButtonElement;
  expect(jsonEdit.disabled).toBe(false);

  // The bespoke permissions/statusLine/sandbox editors ship in Task 13, and the
  // hooks editor ships in Task 14 → all enabled now (no inert object rows remain).
  for (const key of ['permissions', 'statusLine', 'sandbox', 'hooks']) {
    const edit = rowByKey(container, key).querySelector('[data-testid="setting-edit"]') as HTMLButtonElement;
    expect(edit, `${key} Edit… should exist`).toBeTruthy();
    expect(edit.disabled, `${key} Edit… should be enabled`).toBe(false);
  }
});

// ── Task 12: array + env + JSON object editors ───────────────────────────────

test('array editor add/remove entry then save calls set with the new array', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const arrRow = await waitFor(() => rowByKey(container, 'enabledMcpjsonServers'));
  fireEvent.click(arrRow.querySelector('[data-testid="setting-edit"]') as HTMLButtonElement);

  // The modal seeds from the row's current entries (context7, postman).
  const modal = await findByTestId('setting-array-editor');
  const items = () => modal.querySelectorAll('[data-testid="setting-array-item"]');
  expect(items().length).toBe(2);

  // Remove the first entry (context7).
  fireEvent.click(modal.querySelector('[data-testid="setting-array-remove"]') as HTMLButtonElement);
  await waitFor(() => expect(items().length).toBe(1));

  // Add a new entry "x".
  const input = modal.querySelector('[data-testid="setting-array-input"]') as HTMLInputElement;
  fireEvent.input(input, { target: { value: 'x' } });
  fireEvent.click(modal.querySelector('[data-testid="setting-array-add"]') as HTMLButtonElement);
  await waitFor(() => expect(items().length).toBe(2));

  // Save writes the composed string[] (postman + x) to user scope.
  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  expect((api.set as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
    'user', 'enabledMcpjsonServers', 'settings.json', ['postman', 'x'],
  ]);
});

test('env editor add key/value then save calls set with the composed object', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const envRow = await waitFor(() => rowByKey(container, 'env'));
  fireEvent.click(envRow.querySelector('[data-testid="setting-edit"]') as HTMLButtonElement);

  const modal = await findByTestId('setting-env-editor');
  // Add a variable row, then fill its name + value.
  fireEvent.click(modal.querySelector('[data-testid="setting-env-add"]') as HTMLButtonElement);
  const nameInput = await waitFor(() => modal.querySelector('[data-testid="setting-env-name"]') as HTMLInputElement);
  const valueInput = modal.querySelector('[data-testid="setting-env-value"]') as HTMLInputElement;
  fireEvent.input(nameInput, { target: { value: 'API_HOST' } });
  fireEvent.input(valueInput, { target: { value: 'example.com' } });

  // Save composes { name: value } and writes the whole env object.
  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  expect((api.set as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
    'user', 'env', 'settings.json', { API_HOST: 'example.com' },
  ]);
});

test('json editor blocks invalid JSON (error, no set) and saves valid JSON as the parsed object', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const jsonRow = await waitFor(() => rowByKey(container, 'worktree'));
  fireEvent.click(jsonRow.querySelector('[data-testid="setting-edit"]') as HTMLButtonElement);

  const modal = await findByTestId('setting-json-editor');
  const area = modal.querySelector('[data-testid="setting-json-textarea"]') as HTMLTextAreaElement;
  const save = () => modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement;

  // Invalid JSON surfaces an inline error and blocks the write entirely.
  fireEvent.input(area, { target: { value: '{ not valid' } });
  fireEvent.click(save());
  await findByTestId('setting-json-error');
  expect(api.set).not.toHaveBeenCalled();

  // Valid JSON writes the parsed object.
  fireEvent.input(area, { target: { value: '{ "auto": true }' } });
  fireEvent.click(save());
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  expect((api.set as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
    'user', 'worktree', 'settings.json', { auto: true },
  ]);
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

// ── Task 13: bespoke permissions / statusLine / sandbox editors ──────────────

test('permissions editor composes and saves the permissions object (omits empty, preserves unmanaged keys)', async () => {
  const catalog = makeCatalog();
  // Seed a clean slate (no allow/ask/deny) plus a key this editor does NOT
  // manage, to assert both the omit-empty rules and the merge/preserve path.
  const perms = catalog.find((r) => r.def.key === 'permissions')!;
  perms.effective = { defaultMode: 'acceptEdits', disableBypassPermissionsMode: 'disable' };
  const api = makeApi({}, catalog);
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const row = await waitFor(() => rowByKey(container, 'permissions'));
  fireEvent.click(row.querySelector('[data-testid="setting-edit"]') as HTMLButtonElement);

  const modal = await findByTestId('setting-perms-editor');
  // Change defaultMode acceptEdits → plan.
  const dm = modal.querySelector('[data-testid="setting-perms-defaultmode"]') as HTMLSelectElement;
  fireEvent.change(dm, { target: { value: 'plan' } });
  // Add a single deny rule; leave allow / ask / additionalDirectories empty.
  const denyInput = modal.querySelector('[data-testid="setting-list-deny-input"]') as HTMLInputElement;
  fireEvent.input(denyInput, { target: { value: 'Bash(rm *)' } });
  fireEvent.click(modal.querySelector('[data-testid="setting-list-deny-add"]') as HTMLButtonElement);

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  // Empty allow/ask/additionalDirectories are omitted; defaultMode + deny are
  // written; the unmanaged disableBypassPermissionsMode key is preserved.
  expect((api.set as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
    'user', 'permissions', 'settings.json',
    { defaultMode: 'plan', deny: ['Bash(rm *)'], disableBypassPermissionsMode: 'disable' },
  ]);
});

test('statusLine editor saves { type, command } (padding omitted when blank)', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const row = await waitFor(() => rowByKey(container, 'statusLine'));
  fireEvent.click(row.querySelector('[data-testid="setting-edit"]') as HTMLButtonElement);

  const modal = await findByTestId('setting-statusline-editor');
  // type defaults to the fixed "command"; fill the command, leave padding blank.
  const cmd = modal.querySelector('[data-testid="setting-statusline-command"]') as HTMLInputElement;
  fireEvent.input(cmd, { target: { value: '~/bin/statusline.sh' } });

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  expect((api.set as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
    'user', 'statusLine', 'settings.json', { type: 'command', command: '~/bin/statusline.sh' },
  ]);
});

test('statusLine editor omits the command key when the command is blank/whitespace', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const row = await waitFor(() => rowByKey(container, 'statusLine'));
  fireEvent.click(row.querySelector('[data-testid="setting-edit"]') as HTMLButtonElement);

  const modal = await findByTestId('setting-statusline-editor');
  // Type a whitespace-only command, then save. A blank command must NEVER be
  // persisted as `statusLine.command: ""` — Claude Code would try to run it as an
  // empty command. `command` is symmetric with `type`/`padding`: omitted when blank.
  const cmd = modal.querySelector('[data-testid="setting-statusline-command"]') as HTMLInputElement;
  fireEvent.input(cmd, { target: { value: '   ' } });

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  const saved = (api.set as ReturnType<typeof vi.fn>).mock.calls[0][3] as Record<string, unknown>;
  expect(saved).not.toHaveProperty('command');
  // `type` still writes (defaults to the fixed "command"); no leftover command key.
  expect(saved).toEqual({ type: 'command' });
});

test('sandbox editor composes the nested filesystem/network object (omits empty sub-arrays)', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const row = await waitFor(() => rowByKey(container, 'sandbox'));
  fireEvent.click(row.querySelector('[data-testid="setting-edit"]') as HTMLButtonElement);

  const modal = await findByTestId('setting-sandbox-editor');
  // Add one filesystem allow-write path and one network allowed-domain; every
  // other list stays empty and must be omitted.
  const fsInput = modal.querySelector('[data-testid="setting-list-fs-allowWrite-input"]') as HTMLInputElement;
  fireEvent.input(fsInput, { target: { value: '/tmp/build' } });
  fireEvent.click(modal.querySelector('[data-testid="setting-list-fs-allowWrite-add"]') as HTMLButtonElement);
  const netInput = modal.querySelector('[data-testid="setting-list-net-allowedDomains-input"]') as HTMLInputElement;
  fireEvent.input(netInput, { target: { value: 'github.com' } });
  fireEvent.click(modal.querySelector('[data-testid="setting-list-net-allowedDomains-add"]') as HTMLButtonElement);

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  expect((api.set as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
    'user', 'sandbox', 'settings.json',
    { filesystem: { allowWrite: ['/tmp/build'] }, network: { allowedDomains: ['github.com'] } },
  ]);
});

// ── Task 14: bespoke hooks editor (event → matcher-group → command list) ─────

/** Open the hooks editor modal for the `hooks` row and return it. */
async function openHooksEditor(
  container: HTMLElement,
  findByTestId: (id: string) => Promise<HTMLElement>,
): Promise<HTMLElement> {
  const row = await waitFor(() => rowByKey(container, 'hooks'));
  fireEvent.click(row.querySelector('[data-testid="setting-edit"]') as HTMLButtonElement);
  return findByTestId('setting-hooks-editor');
}

/** Add a known event via the picker (select + Add event button). */
function addHookEvent(modal: HTMLElement, event: string) {
  const select = modal.querySelector('[data-testid="hooks-add-event-select"]') as HTMLSelectElement;
  fireEvent.change(select, { target: { value: event } });
  fireEvent.click(modal.querySelector('[data-testid="hooks-add-event"]') as HTMLButtonElement);
}

/** The event `<section>` whose heading names `event`. */
function hookEventSection(modal: HTMLElement, event: string): HTMLElement {
  const sections = Array.from(modal.querySelectorAll('[data-testid="hooks-event"]'));
  const found = sections.find((s) => s.textContent?.includes(event));
  if (!found) throw new Error(`no hooks-event section for "${event}"`);
  return found as HTMLElement;
}

const setCalls = (api: SettingsApi) => (api.set as ReturnType<typeof vi.fn>).mock.calls;

test('hooks editor adds an event+matcher+command and saves the correct nested object', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const modal = await openHooksEditor(container, findByTestId);
  // Empty hooks row → no event sections yet.
  expect(modal.querySelectorAll('[data-testid="hooks-event"]').length).toBe(0);

  // Add PreToolUse; its seeded group exposes a matcher + one command input.
  addHookEvent(modal, 'PreToolUse');
  const section = await waitFor(() => hookEventSection(modal, 'PreToolUse'));
  fireEvent.input(section.querySelector('[data-testid="hooks-matcher"]') as HTMLInputElement, {
    target: { value: 'Bash' },
  });
  fireEvent.input(section.querySelector('[data-testid="hooks-command-input"]') as HTMLInputElement, {
    target: { value: 'echo hi' },
  });

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  // Exact documented shape; no timeout key (it was left blank).
  expect(setCalls(api)[0]).toEqual([
    'user', 'hooks', 'settings.json',
    { PreToolUse: [{ matcher: 'Bash', hooks: [{ type: 'command', command: 'echo hi' }] }] },
  ]);
});

test('hooks editor with a timeout includes it as a number', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const modal = await openHooksEditor(container, findByTestId);
  addHookEvent(modal, 'PostToolUse');
  const section = await waitFor(() => hookEventSection(modal, 'PostToolUse'));
  fireEvent.input(section.querySelector('[data-testid="hooks-matcher"]') as HTMLInputElement, {
    target: { value: 'Write' },
  });
  fireEvent.input(section.querySelector('[data-testid="hooks-command-input"]') as HTMLInputElement, {
    target: { value: 'prettier --write $CLAUDE_FILE' },
  });
  fireEvent.input(section.querySelector('[data-testid="hooks-timeout"]') as HTMLInputElement, {
    target: { value: '30' },
  });

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  // timeout is written as a number, not the raw input string.
  expect(setCalls(api)[0]).toEqual([
    'user', 'hooks', 'settings.json',
    { PostToolUse: [{ matcher: 'Write', hooks: [{ type: 'command', command: 'prettier --write $CLAUDE_FILE', timeout: 30 }] }] },
  ]);
});

test('hooks editor preserves an unmanaged/pre-existing event when editing another', async () => {
  const catalog = makeCatalog();
  // Seed a pre-existing SessionStart hook (matcher absent, as CC allows).
  const hooks = catalog.find((r) => r.def.key === 'hooks')!;
  hooks.effective = { SessionStart: [{ hooks: [{ type: 'command', command: 'echo start' }] }] };
  hooks.isSet = true;
  hooks.sourceScope = 'user';
  const api = makeApi({}, catalog);
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const modal = await openHooksEditor(container, findByTestId);
  // The seeded SessionStart event renders as a section.
  expect(modal.querySelectorAll('[data-testid="hooks-event"]').length).toBe(1);

  // Add and fill a new PreToolUse event, leaving SessionStart untouched.
  addHookEvent(modal, 'PreToolUse');
  const section = await waitFor(() => hookEventSection(modal, 'PreToolUse'));
  fireEvent.input(section.querySelector('[data-testid="hooks-matcher"]') as HTMLInputElement, {
    target: { value: 'Edit' },
  });
  fireEvent.input(section.querySelector('[data-testid="hooks-command-input"]') as HTMLInputElement, {
    target: { value: 'echo edited' },
  });

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  // SessionStart survives the round-trip; PreToolUse is added alongside it.
  expect(setCalls(api)[0]).toEqual([
    'user', 'hooks', 'settings.json',
    {
      SessionStart: [{ hooks: [{ type: 'command', command: 'echo start' }] }],
      PreToolUse: [{ matcher: 'Edit', hooks: [{ type: 'command', command: 'echo edited' }] }],
    },
  ]);
});

test('hooks editor removing a command omits the emptied group and event', async () => {
  const catalog = makeCatalog();
  const hooks = catalog.find((r) => r.def.key === 'hooks')!;
  hooks.effective = { Stop: [{ hooks: [{ type: 'command', command: 'echo bye' }] }] };
  hooks.isSet = true;
  hooks.sourceScope = 'user';
  const api = makeApi({}, catalog);
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const modal = await openHooksEditor(container, findByTestId);
  const section = hookEventSection(modal, 'Stop');
  // Remove the only command → its group has nothing, so the event drops entirely.
  fireEvent.click(section.querySelector('[data-testid="hooks-command-remove"]') as HTMLButtonElement);

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  // Emptied structures cascade-omit: no commands → no group → no event → {}.
  expect(setCalls(api)[0]).toEqual(['user', 'hooks', 'settings.json', {}]);
});

test('hooks editor removing one matcher group keeps the other', async () => {
  const catalog = makeCatalog();
  const hooks = catalog.find((r) => r.def.key === 'hooks')!;
  hooks.effective = {
    PreToolUse: [
      { matcher: 'X', hooks: [{ type: 'command', command: 'a' }] },
      { matcher: 'Y', hooks: [{ type: 'command', command: 'b' }] },
    ],
  };
  hooks.isSet = true;
  hooks.sourceScope = 'user';
  const api = makeApi({}, catalog);
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const modal = await openHooksEditor(container, findByTestId);
  const section = hookEventSection(modal, 'PreToolUse');
  expect(section.querySelectorAll('[data-testid="hooks-group"]').length).toBe(2);
  // Remove the first group (matcher X).
  fireEvent.click(section.querySelector('[data-testid="hooks-remove-group"]') as HTMLButtonElement);
  await waitFor(() => expect(section.querySelectorAll('[data-testid="hooks-group"]').length).toBe(1));

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  expect(setCalls(api)[0]).toEqual([
    'user', 'hooks', 'settings.json',
    { PreToolUse: [{ matcher: 'Y', hooks: [{ type: 'command', command: 'b' }] }] },
  ]);
});

test('hooks editor preserves an http (non-command) hook untouched while editing a command event', async () => {
  const catalog = makeCatalog();
  const hooks = catalog.find((r) => r.def.key === 'hooks')!;
  // An http-only event Ward reads (scan_hooks models type:http+url) plus a normal
  // command event. Opening + saving must NOT destroy the http hook.
  hooks.effective = {
    PostToolUse: [{ hooks: [{ type: 'http', url: 'https://x/y' }] }],
    PreToolUse: [{ matcher: 'Bash', hooks: [{ type: 'command', command: 'echo hi' }] }],
  };
  hooks.isSet = true;
  hooks.sourceScope = 'user';
  const api = makeApi({}, catalog);
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const modal = await openHooksEditor(container, findByTestId);
  // The http-only event renders read-only (a "preserved" chip; no command input).
  const httpSection = hookEventSection(modal, 'PostToolUse');
  expect(httpSection.querySelector('[data-testid="hooks-passthrough"]')).toBeTruthy();
  expect(httpSection.querySelector('[data-testid="hooks-command-input"]')).toBeNull();

  // Edit only the command event.
  const cmdSection = hookEventSection(modal, 'PreToolUse');
  fireEvent.input(cmdSection.querySelector('[data-testid="hooks-command-input"]') as HTMLInputElement, {
    target: { value: 'echo hi2' },
  });

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  // The http hook survives verbatim; the command edit lands.
  expect(setCalls(api)[0]).toEqual([
    'user', 'hooks', 'settings.json',
    {
      PostToolUse: [{ hooks: [{ type: 'http', url: 'https://x/y' }] }],
      PreToolUse: [{ matcher: 'Bash', hooks: [{ type: 'command', command: 'echo hi2' }] }],
    },
  ]);
});

test('hooks editor preserves an http hook interleaved with a command in one matcher-group', async () => {
  const catalog = makeCatalog();
  const hooks = catalog.find((r) => r.def.key === 'hooks')!;
  hooks.effective = {
    PreToolUse: [{
      matcher: 'Bash',
      hooks: [
        { type: 'command', command: 'echo a' },
        { type: 'http', url: 'https://hook/z' },
      ],
    }],
  };
  hooks.isSet = true;
  hooks.sourceScope = 'user';
  const api = makeApi({}, catalog);
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const modal = await openHooksEditor(container, findByTestId);
  const section = hookEventSection(modal, 'PreToolUse');
  // One editable command + one preserved (http) chip in the same group.
  expect(section.querySelectorAll('[data-testid="hooks-command-input"]').length).toBe(1);
  expect(section.querySelectorAll('[data-testid="hooks-passthrough"]').length).toBe(1);

  fireEvent.input(section.querySelector('[data-testid="hooks-command-input"]') as HTMLInputElement, {
    target: { value: 'echo a2' },
  });

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  // Both entries survive, in their original order (command edited, http verbatim).
  expect(setCalls(api)[0]).toEqual([
    'user', 'hooks', 'settings.json',
    {
      PreToolUse: [{
        matcher: 'Bash',
        hooks: [
          { type: 'command', command: 'echo a2' },
          { type: 'http', url: 'https://hook/z' },
        ],
      }],
    },
  ]);
});

test('hooks editor preserves unknown extra fields on a command entry', async () => {
  const catalog = makeCatalog();
  const hooks = catalog.find((r) => r.def.key === 'hooks')!;
  hooks.effective = {
    Stop: [{ hooks: [{ type: 'command', command: 'echo x', timeout: 5, retries: 3 }] }],
  };
  hooks.isSet = true;
  hooks.sourceScope = 'user';
  const api = makeApi({}, catalog);
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const modal = await openHooksEditor(container, findByTestId);
  const section = hookEventSection(modal, 'Stop');
  // Edit only the command text; timeout + the unknown `retries` field must survive.
  fireEvent.input(section.querySelector('[data-testid="hooks-command-input"]') as HTMLInputElement, {
    target: { value: 'echo y' },
  });

  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);
  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  expect(setCalls(api)[0]).toEqual([
    'user', 'hooks', 'settings.json',
    { Stop: [{ hooks: [{ type: 'command', command: 'echo y', timeout: 5, retries: 3 }] }] },
  ]);
});

// ── Task 15: schema-diff panel + env-var panel + polish ──────────────────────

test('schema-diff button calls settingsSchemaDiff and renders the missing keys', async () => {
  const schemaDiff = vi.fn(async (): Promise<SchemaDiff> => ({
    inSchemaNotCatalog: ['someNewKey'],
    inCatalogNotSchema: [],
  }));
  const api = makeApi({ schemaDiff });
  const { findByTestId } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  // Before the click there is no result area — the diff is a user-triggered call.
  const btn = (await findByTestId('settings-schema-diff')) as HTMLButtonElement;
  fireEvent.click(btn);

  await waitFor(() => expect(schemaDiff).toHaveBeenCalledTimes(1));
  const results = await findByTestId('schema-diff-results');
  // The schema key the catalog lacks is surfaced as an "add this" item.
  expect(results.textContent).toContain('someNewKey');
});

test('schema-diff empty result shows an up-to-date message', async () => {
  const schemaDiff = vi.fn(async (): Promise<SchemaDiff> => ({
    inSchemaNotCatalog: [],
    inCatalogNotSchema: [],
  }));
  const api = makeApi({ schemaDiff });
  const { findByTestId } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  fireEvent.click((await findByTestId('settings-schema-diff')) as HTMLButtonElement);
  const results = await findByTestId('schema-diff-results');
  await waitFor(() => expect(results.textContent).toMatch(/up to date/i));
});

test('env panel lists env vars and search filters them', async () => {
  const envList = vi.fn(async (): Promise<EnvVarDef[]> => [
    { name: 'ANTHROPIC_API_KEY', description: 'API key sent as the X-Api-Key header.', category: 'Authentication & Provider' },
    { name: 'HTTP_PROXY', description: 'Proxy server URL for outbound HTTP.', category: 'Config, Paths & Proxy' },
  ]);
  const api = makeApi({ envList });
  const { findByTestId, getByTestId, getAllByTestId } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  await findByTestId('settings-env-list');
  // Both curated vars are listed with name + description + category.
  await waitFor(() => expect(getAllByTestId('env-var-row').length).toBe(2));
  const apiKeyRow = getAllByTestId('env-var-row').find((r) => r.textContent?.includes('ANTHROPIC_API_KEY'))!;
  expect(apiKeyRow.textContent).toContain('Authentication & Provider');

  // A search narrows over name + description.
  const search = getByTestId('env-search') as HTMLInputElement;
  fireEvent.input(search, { target: { value: 'proxy' } });
  await waitFor(() => {
    const rows = getAllByTestId('env-var-row');
    expect(rows.length).toBe(1);
    expect(rows[0].textContent).toContain('HTTP_PROXY');
  });
});

test('env panel "Add to env" opens the env editor seeded with the variable name', async () => {
  const envList = vi.fn(async (): Promise<EnvVarDef[]> => [
    { name: 'HTTP_PROXY', description: 'Proxy server URL for outbound HTTP.', category: 'Config, Paths & Proxy' },
  ]);
  const api = makeApi({ envList });
  const { findByTestId, getAllByTestId } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');
  await findByTestId('settings-env-list');

  const addBtn = (await waitFor(() => getAllByTestId('env-add')))[0] as HTMLButtonElement;
  fireEvent.click(addBtn);

  // The existing env object editor opens, pre-seeded with a row whose NAME is the
  // chosen variable (value blank, ready for the user to fill in).
  const modal = await findByTestId('setting-env-editor');
  const nameInput = modal.querySelector('[data-testid="setting-env-name"]') as HTMLInputElement;
  expect(nameInput.value).toBe('HTTP_PROXY');
});

test('array editor Save folds a half-typed draft into the saved array', async () => {
  const api = makeApi();
  const { findByTestId, container } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const arrRow = await waitFor(() => rowByKey(container, 'enabledMcpjsonServers'));
  fireEvent.click(arrRow.querySelector('[data-testid="setting-edit"]') as HTMLButtonElement);

  const modal = await findByTestId('setting-array-editor');
  // Type into the add-row input but DON'T click Add, then Save directly.
  const input = modal.querySelector('[data-testid="setting-array-input"]') as HTMLInputElement;
  fireEvent.input(input, { target: { value: 'draftval' } });
  fireEvent.click(modal.querySelector('[data-testid="settings-editor-save"]') as HTMLButtonElement);

  await waitFor(() => expect(api.set).toHaveBeenCalledTimes(1));
  // The unadded draft is folded into the composed array (seeded context7/postman + draftval).
  expect((api.set as ReturnType<typeof vi.fn>).mock.calls[0]).toEqual([
    'user', 'enabledMcpjsonServers', 'settings.json', ['context7', 'postman', 'draftval'],
  ]);
});

test('form editor modal does not close on backdrop click', async () => {
  const api = makeApi();
  const { findByTestId, container, queryByTestId } = render(() => <Settings scan={makeHostScan(true)} api={api} />);
  await findByTestId('settings-mode');

  const arrRow = await waitFor(() => rowByKey(container, 'enabledMcpjsonServers'));
  fireEvent.click(arrRow.querySelector('[data-testid="setting-edit"]') as HTMLButtonElement);
  await findByTestId('setting-array-editor');

  // Clicking the dimmed backdrop must NOT dismiss a data-entry modal (unsaved
  // edits would be lost silently) — only Cancel/Esc close it.
  const overlay = await findByTestId('settings-modal-overlay');
  fireEvent.click(overlay);

  expect(queryByTestId('setting-array-editor')).toBeTruthy();
  expect(api.set).not.toHaveBeenCalled();
});
