import { createSignal, createMemo, For, Show } from 'solid-js';
import type { HarnessItem, ScanResult } from '../api';

function effectiveBadge(item: HarnessItem): string | null {
  if (!item.effective) return null;
  if (item.effective === 'shadowed') return '🌫 shadowed';
  if (item.effective === 'conflict') return '⚠ conflict';
  if (item.effective === 'ancestor') return '↑ ancestor';
  return null;
}

export function Organizer(props: { scan: ScanResult; loadFile: (path: string) => Promise<string> }) {
  const [activeCat, setActiveCat] = createSignal(props.scan.categories[0]?.id ?? '');
  const [detail, setDetail] = createSignal<string>('');
  const [selected, setSelected] = createSignal<string>('');
  const [showEffective, setShowEffective] = createSignal(false);

  const itemsForCat = createMemo(() =>
    props.scan.items.filter((i) => i.category === activeCat())
  );

  // When Show Effective is ON, restrict to items that participate in the
  // effective resolution for the most-relevant project scope — i.e. items
  // the framework tagged as shadowed/conflict/ancestor, plus all project-scope
  // items (active winners). Global items without an effective tag are hidden.
  const effectiveKeys = createMemo<Set<string>>(() => {
    const s = new Set<string>();
    for (const item of props.scan.items) {
      // Tag present → shadowed/conflict/ancestor (always include).
      if (item.effective) {
        s.add(`${item.category}::${item.name}::${item.scopeId}`);
        continue;
      }
      // No tag → only include if this is a project (non-global) scope item,
      // which is the active winner for its name.
      if (item.scopeId !== 'global') {
        s.add(`${item.category}::${item.name}::${item.scopeId}`);
      }
    }
    return s;
  });

  const visibleItems = createMemo(() => {
    const all = itemsForCat();
    if (!showEffective()) return all;
    return all.filter((i) => effectiveKeys().has(`${i.category}::${i.name}::${i.scopeId}`));
  });

  async function open(path: string) {
    setSelected(path);
    setDetail(await props.loadFile(path));
  }

  return (
    <div style={{ display: 'flex', height: '100%' }}>
      <div style={{ width: '220px', 'border-right': '1px solid var(--border)', padding: '10px 8px' }}>
        <div style={{ 'font-size': '10px', color: 'var(--text-dim)' }}>Categories</div>
        <For each={props.scan.categories}>
          {(c) => (
            <div onClick={() => setActiveCat(c.id)}
              style={{ display: 'flex', 'justify-content': 'space-between', padding: '5px 8px', margin: '3px 0',
                'border-radius': 'var(--radius)', cursor: 'pointer',
                background: activeCat() === c.id ? 'rgba(48,209,88,0.14)' : 'transparent' }}>
              <span>{c.label}</span><span style={{ color: 'var(--text-dim)' }}>{c.count}</span>
            </div>
          )}
        </For>
      </div>

      <div style={{ width: '320px', 'border-right': '1px solid var(--border)', padding: '10px 8px' }}>
        <label style={{ display: 'flex', 'align-items': 'center', gap: '6px',
          'font-size': '11px', color: 'var(--text-dim)', padding: '4px 0' }}>
          <input
            type="checkbox"
            data-testid="show-effective-toggle"
            checked={showEffective()}
            onInput={(e) => setShowEffective(e.currentTarget.checked)}
          />
          Show Effective
        </label>
        <For each={props.scan.scopes}>
          {(scope) => (
            <>
              <div style={{ 'font-size': '9px', color: 'var(--text-dim)', margin: '6px 0 3px' }}>{scope.label}</div>
              <For each={visibleItems().filter((i) => i.scopeId === scope.id)}>
                {(item) => (
                  <div onClick={() => open(item.path)}
                    data-testid="item-row"
                    data-item-name={item.name}
                    style={{ display: 'flex', 'align-items': 'center', 'justify-content': 'space-between',
                      padding: '5px 8px', margin: '3px 0', 'border-radius': 'var(--radius)', cursor: 'pointer',
                      background: selected() === item.path ? 'var(--surface)' : 'transparent' }}>
                    <span>{item.name}{item.locked ? ' 🔒' : ''}</span>
                    <Show when={effectiveBadge(item)}>
                      <span style={{ 'font-size': '10px', color: 'var(--text-dim)' }}>{effectiveBadge(item)}</span>
                    </Show>
                  </div>
                )}
              </For>
            </>
          )}
        </For>
      </div>

      <div style={{ flex: 1, padding: '12px' }}>
        <Show when={selected()} fallback={<div style={{ color: 'var(--text-dim)' }}>Select an item</div>}>
          <pre style={{ 'font-family': 'var(--font-mono)', 'font-size': '12px', 'white-space': 'pre-wrap' }}>{detail()}</pre>
        </Show>
      </div>
    </div>
  );
}