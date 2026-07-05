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
    <nav class="sidebar" data-testid="sidebar">
      <div class="brand">
        <div class="brand-mark">◆</div>
        <div>
          <div class="brand-name">Ward</div>
          <div class="brand-tag">Config Command Center</div>
          <div class="brand-scope">Claude Code · Codex</div>
        </div>
      </div>

      <div class="side-group">
        <div class="micro">Harness</div>
        <select
          class="harness-select"
          data-testid="harness-select"
          value={props.harness}
          onChange={(e) => props.onSelectHarness(e.currentTarget.value as HarnessId)}
        >
          <For each={HARNESSES}>
            {(h) => <option value={h.id}>{h.icon}  {h.label}</option>}
          </For>
        </select>
      </div>

      <div class="modes">
        <For each={MODES}>
          {(m) => (
            <div
              classList={{ mode: true, active: props.active === m.id }}
              onClick={() => props.onSelect(m.id)}
            >
              <span class="mode-icon">{m.icon}</span>
              <span class="mode-label">{m.label}</span>
            </div>
          )}
        </For>
      </div>

      <div class="sidebar-foot">
        <div class="k">{props.harness === 'codex' ? '~/.codex' : '~/.claude'}</div>
      </div>
    </nav>
  );
}
