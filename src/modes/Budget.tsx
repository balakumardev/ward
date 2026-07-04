import { createMemo, createResource, createSignal, For, Show } from 'solid-js';
import type { BudgetItem, Scope, ScanResult } from '../api';
import { api } from '../api';

/** Plan 06 — Context Budget mode.
 *
 * Big token meter at the top (used / contextLimit) with warning_threshold
 * and max_output annotated. Below it: a per-component breakdown showing
 * how much each piece (system, MCP schemas, CLAUDE.md, always-loaded items)
 * contributes to the total. Bottom: detailed expand/collapse list of every
 * contributing file + item with its token count + measured/estimated tag.
 *
 * The user selects a scope from a dropdown at the top-left. Switching
 * scope re-runs `api.contextBudget` against the new scope_id.
 *
 * Loading state: while the budget resource is fetching we render a small
 * "Computing…" placeholder so the meter doesn't flicker between values.
 */

const CATEGORY_LABELS: Record<string, string> = {
  skill: 'Skill',
  rule: 'Rule',
  command: 'Command',
  agent: 'Agent',
};

function pct(n: number, d: number): number {
  if (d <= 0) return 0;
  return Math.min(100, Math.round((n / d) * 1000) / 10);
}

function fmt(n: number): string {
  return n.toLocaleString();
}

export function Budget(props: { scan: ScanResult; scope: Scope }) {
  const [budget, { refetch }] = createResource(
    () => ({ harness: props.scan.harnessId, scopeId: props.scope.id }),
    ({ harness, scopeId }) => api.contextBudget(harness, scopeId),
  );

  // Per-item collapse state. Empty array = all collapsed.
  const [expanded, setExpanded] = createSignal<Set<string>>(new Set());
  function toggle(id: string) {
    const next = new Set(expanded());
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setExpanded(next);
  }

  const alwaysLoadedByCategory = createMemo(() => {
    const b = budget();
    if (!b) return [] as Array<{ category: string; items: BudgetItem[]; total: number }>;
    const map = new Map<string, BudgetItem[]>();
    for (const it of b.alwaysLoadedItems) {
      const arr = map.get(it.category) ?? [];
      arr.push(it);
      map.set(it.category, arr);
    }
    return [...map.entries()]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([category, items]) => ({
        category,
        items: items.sort((a, b) => b.tokens - a.tokens),
        total: items.reduce((s, i) => s + i.tokens, 0),
      }));
  });

  return (
    <div data-testid="budget-mode" style={{ padding: '16px', 'font-size': '12px' }}>
      <header style={{ display: 'flex', 'align-items': 'baseline', 'justify-content': 'space-between', 'margin-bottom': '12px' }}>
        <div>
          <h2 style={{ margin: '0 0 4px', 'font-size': '14px' }}>Context Budget — {props.scope.label}</h2>
          <div data-testid="budget-method" style={{ color: 'var(--text-dim)', 'font-size': '11px' }}>
            <Show when={budget()} fallback="Computing…">
              {(b) => (
                <span>{b().measured ? 'measured (real tokenizer)' : 'estimated (bytes/4)'} · 200K context model</span>
              )}
            </Show>
          </div>
        </div>
        <button data-testid="budget-recompute" onClick={() => refetch()}
          style={{ padding: '4px 10px', 'font-size': '11px' }}>Recompute</button>
      </header>

      <Show when={budget()} fallback={
        <div data-testid="budget-loading" style={{ color: 'var(--text-dim)', padding: '16px 0' }}>Computing context budget…</div>
      }>
        {(b) => {
          const total = b().used;
          const limit = b().contextLimit;
          const fillPct = pct(total, limit);
          const warnAt = limit - b().warningThreshold;
          const maxOutAt = limit - b().maxOutput;
          const compactAt = limit - b().autocompactBuffer - b().maxOutput;
          const fillColor = total >= compactAt ? 'var(--crit)'
            : total >= maxOutAt ? 'var(--warn)'
            : total >= warnAt ? '#ff9f0a'
            : 'var(--ok)';

          const SYSTEM_ROW: Array<{ key: string; label: string; tokens: number; measured?: boolean }> = [
            { key: 'system', label: 'System (always loaded)', tokens: b().systemLoaded, measured: false },
            { key: 'system-deferred', label: 'System (deferred tools)', tokens: b().systemDeferred, measured: false },
            { key: 'mcp', label: 'MCP tool schemas', tokens: b().mcpSchemas, measured: false },
            { key: 'claudemd', label: `CLAUDE.md${b().claudeMdFiles.length > 1 ? ` (${b().claudeMdFiles.length} files)` : ''}`, tokens: b().claudemd, measured: b().claudeMdFiles.some(f => f.measured) },
          ];
          const always = alwaysLoadedByCategory();

          return (
            <>
              {/* METER */}
              <section data-testid="budget-meter" style={{
                background: 'var(--surface-2)', 'border-radius': 'var(--radius)',
                padding: '12px', 'margin-bottom': '14px',
              }}>
                <div style={{ display: 'flex', 'justify-content': 'space-between', 'margin-bottom': '6px' }}>
                  <span style={{ 'font-weight': 600 }}>Used</span>
                  <span data-testid="budget-used">
                    {fmt(total)} / {fmt(limit)} <span style={{ color: 'var(--text-dim)' }}>({fillPct}%)</span>
                  </span>
                </div>
                <div data-testid="budget-bar-track" style={{
                  position: 'relative', height: '14px',
                  background: 'rgba(255,255,255,0.06)', 'border-radius': '7px', overflow: 'hidden',
                }}>
                  <div data-testid="budget-bar-fill" style={{
                    position: 'absolute', left: '0', top: '0', bottom: '0',
                    width: `${fillPct}%`, background: fillColor, transition: 'width 200ms',
                  }} />
                  {/* Warning threshold marker */}
                  <div title="warning threshold" style={{
                    position: 'absolute', top: '-2px', bottom: '-2px',
                    left: `${pct(limit - b().warningThreshold, limit)}%`, width: '2px', background: '#ff9f0a',
                  }} />
                  {/* Max-output reservation marker */}
                  <div title="max output reservation" style={{
                    position: 'absolute', top: '-2px', bottom: '-2px',
                    left: `${pct(limit - b().maxOutput, limit)}%`, width: '2px', background: 'var(--crit)',
                  }} />
                </div>
                <div style={{ display: 'flex', gap: '12px', 'margin-top': '6px', 'font-size': '11px', color: 'var(--text-dim)' }}>
                  <span><span style={{ color: '#ff9f0a' }}>▌</span> warn @ {fmt(warnAt)}</span>
                  <span><span style={{ color: 'var(--crit)' }}>▌</span> max-out @ {fmt(maxOutAt)}</span>
                  <span>autocompact @ {fmt(compactAt)}</span>
                </div>
              </section>

              {/* COMPONENT BREAKDOWN */}
              <section data-testid="budget-breakdown" style={{ 'margin-bottom': '14px' }}>
                <h3 style={{ 'font-size': '12px', margin: '0 0 6px', color: 'var(--text-dim)' }}>Where the tokens come from</h3>
                <table style={{ width: '100%', 'border-collapse': 'collapse' }}>
                  <thead>
                    <tr style={{ 'text-align': 'left', color: 'var(--text-dim)', 'font-size': '11px' }}>
                      <th style={{ padding: '4px 8px' }}>Component</th>
                      <th style={{ padding: '4px 8px', 'text-align': 'right' }}>Tokens</th>
                      <th style={{ padding: '4px 8px', 'text-align': 'right' }}>Share</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={SYSTEM_ROW}>
                      {(row) => (
                        <tr data-testid={`budget-row-system-${row.key}`}>
                          <td style={{ padding: '4px 8px' }}>{row.label}</td>
                          <td style={{ padding: '4px 8px', 'text-align': 'right' }}>{fmt(row.tokens)}</td>
                          <td style={{ padding: '4px 8px', 'text-align': 'right', color: 'var(--text-dim)' }}>
                            {pct(row.tokens, total)}%
                          </td>
                        </tr>
                      )}
                    </For>
                    <For each={always}>
                      {(group) => (
                        <tr data-testid={`budget-row-always-${group.category}`}>
                          <td style={{ padding: '4px 8px' }}>{CATEGORY_LABELS[group.category] ?? group.category} ({group.items.length})</td>
                          <td style={{ padding: '4px 8px', 'text-align': 'right' }}>{fmt(group.total)}</td>
                          <td style={{ padding: '4px 8px', 'text-align': 'right', color: 'var(--text-dim)' }}>
                            {pct(group.total, total)}%
                          </td>
                        </tr>
                      )}
                    </For>
                    <tr style={{ 'border-top': '1px solid var(--border)' }}>
                      <td style={{ padding: '6px 8px', 'font-weight': 600 }}>Total used</td>
                      <td data-testid="budget-total" style={{ padding: '6px 8px', 'text-align': 'right', 'font-weight': 600 }}>{fmt(total)}</td>
                      <td style={{ padding: '6px 8px', 'text-align': 'right' }}>100%</td>
                    </tr>
                  </tbody>
                </table>
              </section>

              {/* DETAIL LIST */}
              <section data-testid="budget-detail">
                <h3 style={{ 'font-size': '12px', margin: '0 0 6px', color: 'var(--text-dim)' }}>Files &amp; items</h3>
                <Show when={b().claudeMdFiles.length > 0}>
                  <div style={{ 'margin-bottom': '8px' }}>
                    <div style={{ 'font-weight': 500, 'margin-bottom': '4px' }}>CLAUDE.md</div>
                    <For each={b().claudeMdFiles}>
                      {(f) => (
                        <div data-testid="budget-claudemd-file" style={{
                          display: 'flex', 'justify-content': 'space-between', gap: '8px',
                          padding: '4px 8px', background: 'var(--surface-2)', 'border-radius': 'var(--radius)',
                          'margin-bottom': '4px',
                        }}>
                          <span style={{ 'min-width': 0, overflow: 'hidden', 'text-overflow': 'ellipsis', 'white-space': 'nowrap' }}>
                            <code>{f.name}</code> <span style={{ color: 'var(--text-dim)' }}>· {f.path}</span>
                          </span>
                          <span style={{ 'white-space': 'nowrap' }}>
                            <strong>{fmt(f.tokens)}</strong> <span style={{ color: 'var(--text-dim)' }}>{f.measured ? 'measured' : 'estimated'}</span>
                          </span>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
                <Show when={always.length > 0}>
                  <div>
                    <div style={{ 'font-weight': 500, 'margin-bottom': '4px' }}>Always-loaded items</div>
                    <For each={always}>
                      {(group) => (
                        <div data-testid={`budget-group-${group.category}`} style={{ 'margin-bottom': '8px' }}>
                          <div
                            onClick={() => toggle(`g-${group.category}`)}
                            style={{
                              cursor: 'pointer', padding: '4px 8px',
                              background: 'var(--surface-2)', 'border-radius': 'var(--radius)',
                              display: 'flex', 'justify-content': 'space-between',
                            }}
                          >
                            <span>
                              {expanded().has(`g-${group.category}`) ? '▾' : '▸'} {CATEGORY_LABELS[group.category] ?? group.category} ({group.items.length})
                            </span>
                            <span><strong>{fmt(group.total)}</strong></span>
                          </div>
                          <Show when={expanded().has(`g-${group.category}`)}>
                            <For each={group.items}>
                              {(it) => (
                                <div data-testid="budget-item-row" style={{
                                  display: 'flex', 'justify-content': 'space-between', gap: '8px',
                                  padding: '3px 8px 3px 24px', color: 'var(--text-dim)',
                                }}>
                                  <span style={{ 'min-width': 0, overflow: 'hidden', 'text-overflow': 'ellipsis', 'white-space': 'nowrap' }}>{it.name}</span>
                                  <span style={{ 'white-space': 'nowrap' }}>
                                    <strong style={{ color: 'var(--text)' }}>{fmt(it.tokens)}</strong> {it.measured ? 'measured' : 'estimated'}
                                  </span>
                                </div>
                              )}
                            </For>
                          </Show>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
                <Show when={b().claudeMdFiles.length === 0 && always.length === 0}>
                  <div data-testid="budget-empty" style={{ color: 'var(--text-dim)', padding: '8px' }}>
                    No always-loaded items or CLAUDE.md files in this scope.
                  </div>
                </Show>
              </section>
            </>
          );
        }}
      </Show>
    </div>
  );
}

/** Lightweight wrapper that pairs Budget with a scope picker. App.tsx
 *  uses this when the user enters Budget mode without a pre-selected
 *  scope. */
export function BudgetWithPicker(props: { scan: ScanResult; initialScopeId?: string }) {
  const [scopeId, setScopeId] = createSignal(props.initialScopeId ?? props.scan.scopes[0]?.id ?? '');
  const scope = createMemo(() => props.scan.scopes.find(s => s.id === scopeId()));
  return (
    <div style={{ display: 'flex', height: '100%', 'font-size': '12px' }}>
      <aside style={{
        width: '210px', 'border-right': '1px solid var(--border)',
        padding: '8px', 'overflow-y': 'auto', background: 'var(--surface-2)',
      }}>
        <div style={{ 'font-size': '11px', color: 'var(--text-dim)', 'margin-bottom': '6px' }}>Scope</div>
        <For each={props.scan.scopes}>
          {(s) => (
            <div
              data-testid="budget-scope-row"
              data-scope-id={s.id}
              onClick={() => setScopeId(s.id)}
              style={{
                padding: '5px 8px', margin: '2px 0', 'border-radius': 'var(--radius)',
                cursor: 'pointer',
                background: scopeId() === s.id ? 'rgba(48,209,88,0.14)' : 'transparent',
                color: scopeId() === s.id ? 'var(--accent)' : 'var(--text)',
              }}
            >
              {s.label}
            </div>
          )}
        </For>
      </aside>
      <main style={{ flex: 1, 'overflow-y': 'auto' }}>
        <Show when={scope()} fallback={
          <div style={{ padding: '16px', color: 'var(--text-dim)' }}>Select a scope to view its context budget.</div>
        }>
          {(s) => <Budget scan={props.scan} scope={s()} />}
        </Show>
      </main>
    </div>
  );
}
