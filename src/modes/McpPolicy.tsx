import { createSignal, For, Show } from 'solid-js';
import type { McpPolicy, PolicyEntry, RestoreInfo } from '../api';

/** UI for editing the user-scope MCP allowlist/denylist. Saves through
 *  `props.onSave` and stores the returned RestoreInfo via
 *  `props.onUndoAvailable` so the parent (Organizer) can offer Undo. */
export function McpPolicy(props: {
  initial: McpPolicy;
  onSave: (policy: McpPolicy) => Promise<RestoreInfo>;
  onUndoAvailable: (info: RestoreInfo) => void;
  onClose: () => void;
}) {
  const [allowlist, setAllowlist] = createSignal<PolicyEntry[]>([...props.initial.allowlist]);
  const [denylist, setDenylist] = createSignal<PolicyEntry[]>([...props.initial.denylist]);
  const [statusMsg, setStatusMsg] = createSignal<string>('');

  // ── Add / remove entry ──

  function addEntry(list: 'allow' | 'deny') {
    const newEntry: PolicyEntry = { serverName: '' };
    if (list === 'allow') {
      setAllowlist([...allowlist(), newEntry]);
    } else {
      setDenylist([...denylist(), newEntry]);
    }
  }

  function removeEntry(list: 'allow' | 'deny', idx: number) {
    if (list === 'allow') {
      setAllowlist(allowlist().filter((_, i) => i !== idx));
    } else {
      setDenylist(denylist().filter((_, i) => i !== idx));
    }
  }

  function updateEntry(list: 'allow' | 'deny', idx: number, patch: Partial<PolicyEntry>) {
    const update = (e: PolicyEntry): PolicyEntry => ({ ...e, ...patch });
    if (list === 'allow') {
      setAllowlist(allowlist().map((e, i) => (i === idx ? update(e) : e)));
    } else {
      setDenylist(denylist().map((e, i) => (i === idx ? update(e) : e)));
    }
  }

  function setEntryKind(list: 'allow' | 'deny', idx: number, kind: 'name' | 'command' | 'url') {
    const empty: PolicyEntry = kind === 'command' ? { serverCommand: ['', ''] }
      : kind === 'url' ? { serverUrl: '' }
      : { serverName: '' };
    if (list === 'allow') {
      setAllowlist(allowlist().map((e, i) => (i === idx ? empty : e)));
    } else {
      setDenylist(denylist().map((e, i) => (i === idx ? empty : e)));
    }
  }

  function entryKind(e: PolicyEntry): 'name' | 'command' | 'url' {
    if (e.serverCommand) return 'command';
    if (e.serverUrl !== undefined) return 'url';
    return 'name';
  }

  // ── Save ──

  async function save() {
    const policy: McpPolicy = { allowlist: allowlist(), denylist: denylist() };
    try {
      const info = await props.onSave(policy);
      props.onUndoAvailable(info);
      setStatusMsg('Saved. Click Undo in the Organizer to reverse.');
    } catch (e) {
      setStatusMsg(`save failed: ${String(e)}`);
    }
  }

  return (
    <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.45)',
      display: 'flex', 'align-items': 'center', 'justify-content': 'center', 'z-index': 100 }}>
      <div data-testid="mcp-policy-panel" data-policy-kind="mcp"
        style={{ background: 'var(--surface)', border: '1px solid var(--border)',
          'border-radius': 'var(--radius)', padding: '16px', 'min-width': '520px',
          'max-width': '90vw', 'max-height': '80vh', overflow: 'auto' }}>
        <div style={{ display: 'flex', 'align-items': 'center', 'margin-bottom': '12px' }}>
          <strong>MCP Policy</strong>
          <span style={{ flex: 1 }} />
          <button data-testid="mcp-policy-close" onClick={() => props.onClose()}
            style={{ padding: '3px 8px', 'font-size': '11px' }}>Close</button>
        </div>

        <div style={{ 'font-size': '11px', color: 'var(--text-dim)', 'margin-bottom': '12px' }}>
          Saved to <code>~/.claude/settings.json</code> as
          <code> allowedMcpServers</code> / <code>deniedMcpServers</code>.
          Denylist takes absolute precedence. Use <code>*</code> in URLs
          for wildcards (e.g. <code>https://*.evil.com/*</code>).
        </div>

        <PolicyList
          title="Allowlist"
          testidPrefix="allowlist"
          entries={allowlist()}
          onAdd={() => addEntry('allow')}
          onRemove={(idx) => removeEntry('allow', idx)}
          onUpdate={(idx, patch) => updateEntry('allow', idx, patch)}
          onKind={(idx, kind) => setEntryKind('allow', idx, kind)}
          kindFor={entryKind}
        />

        <div style={{ height: '12px' }} />

        <PolicyList
          title="Denylist"
          testidPrefix="denylist"
          entries={denylist()}
          onAdd={() => addEntry('deny')}
          onRemove={(idx) => removeEntry('deny', idx)}
          onUpdate={(idx, patch) => updateEntry('deny', idx, patch)}
          onKind={(idx, kind) => setEntryKind('deny', idx, kind)}
          kindFor={entryKind}
        />

        <div style={{ display: 'flex', 'align-items': 'center', 'margin-top': '16px', gap: '8px' }}>
          <button data-testid="mcp-policy-save" onClick={save}
            style={{ padding: '6px 14px', 'font-size': '13px' }}>Save</button>
          <span style={{ 'font-size': '11px', color: 'var(--text-dim)' }}>{statusMsg()}</span>
        </div>
      </div>
    </div>
  );
}

function PolicyList(props: {
  title: string;
  testidPrefix: string;
  entries: PolicyEntry[];
  onAdd: () => void;
  onRemove: (idx: number) => void;
  onUpdate: (idx: number, patch: Partial<PolicyEntry>) => void;
  onKind: (idx: number, kind: 'name' | 'command' | 'url') => void;
  kindFor: (e: PolicyEntry) => 'name' | 'command' | 'url';
}) {
  return (
    <div>
      <div style={{ display: 'flex', 'align-items': 'center', 'margin-bottom': '6px' }}>
        <strong style={{ 'font-size': '12px' }}>{props.title}</strong>
        <span style={{ flex: 1 }} />
        <button data-testid={`${props.testidPrefix}-add`} onClick={props.onAdd}
          style={{ padding: '2px 8px', 'font-size': '11px' }}>+ Add</button>
      </div>
      <For each={props.entries}>
        {(entry, idx) => {
          const kind = () => props.kindFor(entry);
          return (
            <div data-testid={`${props.testidPrefix}-row`} data-row-index={idx()}
              style={{ display: 'flex', gap: '4px', 'align-items': 'center',
                padding: '4px 0', 'border-bottom': '1px solid var(--border)' }}>
              <select data-testid={`${props.testidPrefix}-kind`}
                value={kind()}
                onChange={(e) => props.onKind(idx(), e.currentTarget.value as 'name' | 'command' | 'url')}
                style={{ 'font-size': '11px' }}>
                <option value="name">name</option>
                <option value="command">command</option>
                <option value="url">url</option>
              </select>
              <Show when={kind() === 'name'}>
                <input data-testid={`${props.testidPrefix}-name`}
                  type="text" value={entry.serverName ?? ''}
                  placeholder="server name"
                  onInput={(e) => props.onUpdate(idx(), { serverName: e.currentTarget.value })}
                  style={{ flex: 1, padding: '2px 4px', 'font-size': '11px',
                    'font-family': 'var(--font-mono)' }} />
              </Show>
              <Show when={kind() === 'command'}>
                <input data-testid={`${props.testidPrefix}-cmd0`}
                  type="text" value={entry.serverCommand?.[0] ?? ''}
                  placeholder="command (e.g. python)"
                  onInput={(e) => props.onUpdate(idx(), {
                    serverCommand: [e.currentTarget.value, ...(entry.serverCommand?.slice(1) ?? [''])]
                  })}
                  style={{ flex: 1, padding: '2px 4px', 'font-size': '11px',
                    'font-family': 'var(--font-mono)' }} />
                <input data-testid={`${props.testidPrefix}-cmd1`}
                  type="text" value={entry.serverCommand?.[1] ?? ''}
                  placeholder="leading arg (e.g. evil.py)"
                  onInput={(e) => props.onUpdate(idx(), {
                    serverCommand: [entry.serverCommand?.[0] ?? '', e.currentTarget.value]
                  })}
                  style={{ flex: 1, padding: '2px 4px', 'font-size': '11px',
                    'font-family': 'var(--font-mono)' }} />
              </Show>
              <Show when={kind() === 'url'}>
                <input data-testid={`${props.testidPrefix}-url`}
                  type="text" value={entry.serverUrl ?? ''}
                  placeholder="url glob (e.g. https://*.evil.com/*)"
                  onInput={(e) => props.onUpdate(idx(), { serverUrl: e.currentTarget.value })}
                  style={{ flex: 1, padding: '2px 4px', 'font-size': '11px',
                    'font-family': 'var(--font-mono)' }} />
              </Show>
              <button data-testid={`${props.testidPrefix}-remove`}
                onClick={() => props.onRemove(idx())}
                style={{ padding: '2px 8px', 'font-size': '11px', color: 'var(--danger)' }}>
                ×
              </button>
            </div>
          );
        }}
      </For>
      <Show when={props.entries.length === 0}>
        <div data-testid={`${props.testidPrefix}-empty`}
          style={{ 'font-size': '11px', color: 'var(--text-dim)', padding: '6px 0' }}>
          (empty)
        </div>
      </Show>
    </div>
  );
}
