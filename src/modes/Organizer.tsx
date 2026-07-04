import { createSignal, createMemo, For, Show } from 'solid-js';
import type { ScanResult } from '../api';

export function Organizer(props: { scan: ScanResult; loadFile: (path: string) => Promise<string> }) {
  const [activeCat, setActiveCat] = createSignal(props.scan.categories[0]?.id ?? '');
  const [detail, setDetail] = createSignal<string>('');
  const [selected, setSelected] = createSignal<string>('');

  const itemsForCat = createMemo(() =>
    props.scan.items.filter((i) => i.category === activeCat())
  );

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

      <div style={{ width: '300px', 'border-right': '1px solid var(--border)', padding: '10px 8px' }}>
        <For each={props.scan.scopes}>
          {(scope) => (
            <>
              <div style={{ 'font-size': '9px', color: 'var(--text-dim)', margin: '6px 0 3px' }}>{scope.label}</div>
              <For each={itemsForCat().filter((i) => i.scopeId === scope.id)}>
                {(item) => (
                  <div onClick={() => open(item.path)}
                    style={{ padding: '5px 8px', margin: '3px 0', 'border-radius': 'var(--radius)', cursor: 'pointer',
                      background: selected() === item.path ? 'var(--surface)' : 'transparent' }}>
                    {item.name}{item.locked ? ' 🔒' : ''}
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
