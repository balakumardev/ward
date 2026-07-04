import { For } from 'solid-js';

export const MODES = [
  { id: 'organizer', label: 'Organizer', icon: '⌘' },
  { id: 'security', label: 'Security', icon: '⛨' },
  { id: 'budget', label: 'Context Budget', icon: '▣' },
  { id: 'sessions', label: 'Sessions', icon: '⧉' },
  { id: 'backups', label: 'Backups', icon: '↺' },
] as const;

export function Sidebar(props: { active: string; onSelect: (id: string) => void }) {
  return (
    <nav style={{ width: '210px', background: 'var(--surface-2)', 'border-right': '1px solid var(--border)', padding: '10px 8px' }}>
      <div style={{ 'font-size': '11px', color: 'var(--text-dim)', margin: '0 6px 8px' }}>◆ Claude Code</div>
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
