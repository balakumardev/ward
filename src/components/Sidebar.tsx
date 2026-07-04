import { For } from 'solid-js';

export const MODES = [
  { id: 'organizer', label: 'Organizer', icon: '⌘' },
  { id: 'security', label: 'Security', icon: '⛨' },
  { id: 'budget', label: 'Context Budget', icon: '▣' },
  { id: 'sessions', label: 'Sessions', icon: '⧉' },
  { id: 'backups', label: 'Backups', icon: '↺' },
] as const;

/** Plan 09 — Harness dropdown options. Order matches `commands.rs::build_registry`. */
export const HARNESSES = [
  { id: 'claude', label: 'Claude Code', icon: '◆' },
  { id: 'codex',  label: 'Codex CLI',   icon: '◇' },
] as const;
export type HarnessId = typeof HARNESSES[number]['id'];

export function Sidebar(props: {
  active: string;
  onSelect: (id: string) => void;
  harness: HarnessId;
  onSelectHarness: (id: HarnessId) => void;
}) {
  return (
    <nav
      data-testid="sidebar"
      style={{
        width: '210px', background: 'var(--surface-2)',
        'border-right': '1px solid var(--border)', padding: '10px 8px',
      }}
    >
      <div style={{ display: 'flex', 'align-items': 'center', gap: '6px', margin: '0 6px 8px' }}>
        <span style={{ 'font-size': '11px', color: 'var(--text-dim)' }}>Harness</span>
        <select
          data-testid="harness-select"
          value={props.harness}
          onChange={(e) => props.onSelectHarness(e.currentTarget.value as HarnessId)}
          style={{
            'font-size': '12px',
            padding: '2px 6px',
            'border-radius': 'var(--radius)',
            border: '1px solid var(--border)',
            background: 'var(--surface)',
            color: 'var(--text)',
          }}
        >
          <For each={HARNESSES}>
            {(h) => <option value={h.id}>{h.icon} {h.label}</option>}
          </For>
        </select>
      </div>
      <For each={MODES}>
        {(m) => (
          <div
            onClick={() => props.onSelect(m.id)}
            style={{
              padding: '5px 8px', margin: '3px 0', 'border-radius': 'var(--radius)', cursor: 'pointer',
              background: props.active === m.id ? 'rgba(48,209,88,0.14)' : 'transparent',
              color: props.active === m.id ? 'var(--accent)' : 'var(--text)',
            }}
          >
            {m.icon} <span>{m.label}</span>
          </div>
        )}
      </For>
    </nav>
  );
}