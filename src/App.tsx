import { createResource, createSignal, Show } from 'solid-js';
import { Shell } from './components/Shell';
import { Organizer } from './modes/Organizer';
import { McpPolicy } from './modes/McpPolicy';
import { Security } from './modes/Security';
import { BudgetWithPicker } from './modes/Budget';
import { Sessions } from './modes/Sessions';
import { Backups } from './modes/Backups';
import { api } from './api';
import type { McpPolicy as McpPolicyType, RestoreInfo } from './api';

export default function App() {
  const [mode, setMode] = createSignal('organizer');
  const [scan, { refetch }] = createResource(() => api.scan('claude'));
  const [showPolicyPanel, setShowPolicyPanel] = createSignal(false);
  // Per Plan 04, the undo state lives in Organizer.tsx (it owns the detail
  // pane that surfaces the Undo button). The McpPolicy panel still emits
  // `onUndoAvailable` so a future "combined" panel can wire the same state
  // back through, but App.tsx intentionally doesn't track it today.
  const [, setLastUndo] = createSignal<RestoreInfo | null>(null);
  const [policyResource, { refetch: refetchPolicy }] = createResource(() => api.mcpGetPolicy());

  // Bridge api → organizer-shape. We re-scan after every mutation so
  // the UI reflects the new on-disk state.
  const organizerApi = {
    listDestinations: (item: Parameters<typeof api.listDestinations>[1]) =>
      api.listDestinations('claude', item),
    moveItem: async (item: Parameters<typeof api.moveItem>[1], destScopeId: string) => {
      const r = await api.moveItem('claude', item, destScopeId);
      await refetch();
      return r;
    },
    deleteItem: async (item: Parameters<typeof api.deleteItem>[1]) => {
      const r = await api.deleteItem('claude', item);
      await refetch();
      return r;
    },
    restore: async (info: Parameters<typeof api.restore>[1]) => {
      await api.restore('claude', info);
      await refetch();
      // Plan 04 — policy changes don't show up in the scan, but the
      // disabled list does; refresh policy so any verdict changes are
      // visible immediately.
      await refetchPolicy();
    },
    bulkRestore: async (infos: Parameters<typeof api.bulkRestore>[1]) => {
      await api.bulkRestore('claude', infos);
      await refetch();
    },
    saveFile: async (path: string, content: string) => {
      await api.saveFile(path, content);
      await refetch();
    },
    bulk: async (items: Parameters<typeof api.bulk>[1], op: 'move' | 'delete', destScopeId?: string) => {
      const r = await api.bulk('claude', items, op, destScopeId);
      await refetch();
      return r;
    },
    // Plan 04 — MCP controls.
    mcpGetDisabled: (projectPath: string) => api.mcpGetDisabled(projectPath),
    mcpSetDisabled: async (projectPath: string, list: string[]) => {
      const r = await api.mcpSetDisabled(projectPath, list);
      await refetch();
      return r;
    },
    mcpGetPolicy: () => api.mcpGetPolicy(),

    // Plan 07 — Sessions mode.
    sessionPreview: (path: string) => api.sessionPreview(path),
    sessionCost: (path: string) => api.sessionCost(path),
    sessionDistill: (path: string) => api.sessionDistill(path),
    sessionTrim: (path: string) => api.sessionTrim(path),
    // `restore` already wired above.

    // Plan 08 — Backup Center.
    backupStatus: () => api.backupStatus(),
    backupRun: (scan: Parameters<typeof api.backupRun>[0], remoteUrl?: string | null) =>
      api.backupRun(scan, remoteUrl),
    backupSync: () => api.backupSync(),
    backupPush: () => api.backupPush(),
    backupSchedulerInstall: (secs: number) => api.backupSchedulerInstall(secs),
    backupSchedulerRemove: () => api.backupSchedulerRemove(),
    backupSetRemote: (url: string) => api.backupSetRemote(url),
  };

  return (
    <Shell active={mode()} onSelect={setMode}>
      <Show when={scan()} fallback={<div style={{ padding: '16px' }}>Scanning ~/.claude…</div>}>
        {(result) => (
          <Show when={mode() === 'organizer'} fallback={
            <Show when={mode() === 'security'} fallback={
              <Show when={mode() === 'budget'} fallback={
                <Show when={mode() === 'sessions'} fallback={
                  <Show when={mode() === 'backups'} fallback={
                    <div style={{ padding: '16px', color: 'var(--text-dim)' }}>Coming in a later plan.</div>
                  }>
                    <Show when={result().capabilities.backup} fallback={
                      <div data-testid="backups-unsupported" style={{ padding: '16px', color: 'var(--text-dim)' }}>
                        Backups mode is not supported by this harness.
                      </div>
                    }>
                      <Backups scan={result()} api={organizerApi as never} />
                    </Show>
                  </Show>
                }>
                  <Show when={result().capabilities.sessions} fallback={
                    <div data-testid="sessions-unsupported" style={{ padding: '16px', color: 'var(--text-dim)' }}>
                      Sessions mode is not supported by this harness.
                    </div>
                  }>
                    <Sessions scan={result()} api={organizerApi as never} />
                  </Show>
                </Show>
              }>
                <Show when={result().capabilities.contextBudget} fallback={
                  <div data-testid="budget-unsupported" style={{ padding: '16px', color: 'var(--text-dim)' }}>
                    Context Budget mode is not supported by this harness.
                  </div>
                }>
                  <BudgetWithPicker scan={result()} />
                </Show>
              </Show>
            }>
              <Security
                items={result().items}
                api={{
                  listDestinations: (item) => api.listDestinations('claude', item) as any,
                  moveItem: (item, dest) => api.moveItem('claude', item, dest),
                  deleteItem: (item) => api.deleteItem('claude', item),
                  restore: (info) => api.restore('claude', info),
                  mcpSetDisabled: (path, list) => api.mcpSetDisabled(path, list),
                }}
              />
            </Show>
          }>
            <div style={{ position: 'relative', height: '100%' }}>
              <div style={{ position: 'absolute', top: '8px', right: '12px', 'z-index': 5 }}>
                <Show when={result().capabilities.mcpPolicy}>
                  <button data-testid="mcp-policy-button"
                    onClick={() => setShowPolicyPanel(true)}
                    style={{ padding: '4px 10px', 'font-size': '11px' }}>
                    MCP Policy
                  </button>
                </Show>
              </div>
              <Organizer scan={result()} loadFile={api.readFileContent} api={organizerApi as never} />
            </div>
            <Show when={showPolicyPanel() && policyResource()}>
              {(policy) => (
                <McpPolicy
                  initial={policy() as McpPolicyType}
                  onSave={async (p) => {
                    const info = await api.mcpSetPolicy(p);
                    await refetchPolicy();
                    return info;
                  }}
                  onUndoAvailable={(info) => setLastUndo(info)}
                  onClose={() => setShowPolicyPanel(false)}
                />
              )}
            </Show>
          </Show>
        )}
      </Show>
    </Shell>
  );
}