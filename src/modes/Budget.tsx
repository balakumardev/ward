import { createMemo, createResource, createSignal, For, Show } from 'solid-js';
import type { BudgetItem, Scope, ScanResult } from '../api';
import { api } from '../api';
import '../styles/budget.css';

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
    <div data-testid="budget-mode" class="bud-mode">
      <header class="bud-header">
        <div>
          <h2 class="bud-title">Context Budget — {props.scope.label}</h2>
          <div data-testid="budget-method" class="bud-method">
            <Show when={budget()} fallback="Computing…">
              {(b) => (
                <span>{b().measured ? 'measured (real tokenizer)' : 'estimated (bytes/4)'} · 200K context model</span>
              )}
            </Show>
          </div>
        </div>
        <button data-testid="budget-recompute" class="btn btn-ghost bud-recompute" onClick={() => refetch()}>Recompute</button>
      </header>

      <Show when={budget()} fallback={
        <div data-testid="budget-loading" class="bud-loading">Computing context budget…</div>
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
          // Presentation-only state derived from the identical thresholds
          // above: healthy gets the mint→teal gradient, warn/crit their
          // solid ramps. (var(--warn) and #ff9f0a are the same orange.)
          const fillState = fillColor === 'var(--crit)' ? 'crit'
            : fillColor === 'var(--ok)' ? 'ok'
            : 'warn';

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
              <section data-testid="budget-meter" class="bud-card bud-meter">
                <div class="bud-meter-top">
                  <div>
                    <div class="bud-eyebrow">Used</div>
                    <div data-testid="budget-used" class="bud-readout">
                      {fmt(total)} / {fmt(limit)} <span class="bud-pct">({fillPct}%)</span>
                    </div>
                  </div>
                  <Show
                    when={b().measured}
                    fallback={<span class="badge badge-warn bud-badge">estimated</span>}
                  >
                    <span class="badge badge-ok bud-badge">measured</span>
                  </Show>
                </div>
                <div data-testid="budget-bar-track" class="bud-track">
                  <div
                    data-testid="budget-bar-fill"
                    classList={{ 'bud-fill': true, ok: fillState === 'ok', warn: fillState === 'warn', crit: fillState === 'crit' }}
                    style={{ width: `${fillPct}%` }}
                  />
                  {/* Warning threshold marker */}
                  <div class="bud-marker warn" title="warning threshold" style={{ left: `${pct(limit - b().warningThreshold, limit)}%` }} />
                  {/* Max-output reservation marker */}
                  <div class="bud-marker crit" title="max output reservation" style={{ left: `${pct(limit - b().maxOutput, limit)}%` }} />
                </div>
                <div class="bud-legend">
                  <span><span class="glyph" style={{ color: '#ff9f0a' }}>▌</span> warn @ {fmt(warnAt)}</span>
                  <span><span class="glyph" style={{ color: 'var(--crit)' }}>▌</span> max-out @ {fmt(maxOutAt)}</span>
                  <span>autocompact @ {fmt(compactAt)}</span>
                </div>
              </section>

              {/* COMPONENT BREAKDOWN */}
              <section data-testid="budget-breakdown" class="bud-card bud-breakdown">
                <h3 class="bud-section-title">Where the tokens come from</h3>
                <div class="bud-bd-list">
                  <div class="bud-bd-headrow">
                    <span>Component</span>
                    <span class="bud-bd-head-num">Tokens</span>
                    <span class="bud-bd-head-share">Share</span>
                  </div>
                  <For each={SYSTEM_ROW}>
                    {(row) => (
                      <div data-testid={`budget-row-system-${row.key}`} class="bud-bd-row">
                        <span class="bud-bd-label">{row.label}</span>
                        <span class="bud-bd-num">{fmt(row.tokens)}</span>
                        <span class="bud-bd-share">
                          <span class="bud-prop"><span class="bud-prop-fill" style={{ width: `${pct(row.tokens, total)}%` }} /></span>
                          <span class="bud-bd-pct">{pct(row.tokens, total)}%</span>
                        </span>
                      </div>
                    )}
                  </For>
                  <For each={always}>
                    {(group) => (
                      <div data-testid={`budget-row-always-${group.category}`} class="bud-bd-row">
                        <span class="bud-bd-label">{CATEGORY_LABELS[group.category] ?? group.category} ({group.items.length})</span>
                        <span class="bud-bd-num">{fmt(group.total)}</span>
                        <span class="bud-bd-share">
                          <span class="bud-prop"><span class="bud-prop-fill" style={{ width: `${pct(group.total, total)}%` }} /></span>
                          <span class="bud-bd-pct">{pct(group.total, total)}%</span>
                        </span>
                      </div>
                    )}
                  </For>
                  <div class="bud-bd-row bud-bd-total">
                    <span class="bud-bd-label">Total used</span>
                    <span data-testid="budget-total" class="bud-bd-num">{fmt(total)}</span>
                    <span class="bud-bd-share"><span class="bud-bd-pct">100%</span></span>
                  </div>
                </div>
              </section>

              {/* DETAIL LIST */}
              <section data-testid="budget-detail" class="bud-card bud-detail">
                <h3 class="bud-section-title">Files &amp; items</h3>
                <Show when={b().claudeMdFiles.length > 0}>
                  <div class="bud-sub">
                    <div class="bud-sub-title">CLAUDE.md</div>
                    <For each={b().claudeMdFiles}>
                      {(f) => (
                        <div data-testid="budget-claudemd-file" class="bud-file-row">
                          <span class="bud-file-name">
                            <code>{f.name}</code> <span class="bud-file-path">· {f.path}</span>
                          </span>
                          <span class="bud-file-meta">
                            <strong>{fmt(f.tokens)}</strong> <span classList={{ 'bud-tag': true, measured: f.measured, estimated: !f.measured }}>{f.measured ? 'measured' : 'estimated'}</span>
                          </span>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
                <Show when={always.length > 0}>
                  <div class="bud-sub">
                    <div class="bud-sub-title">Always-loaded items</div>
                    <For each={always}>
                      {(group) => (
                        <div data-testid={`budget-group-${group.category}`} class="bud-group">
                          <div class="bud-group-head" onClick={() => toggle(`g-${group.category}`)}>
                            <span class="bud-group-name">
                              <span class="bud-caret">{expanded().has(`g-${group.category}`) ? '▾' : '▸'}</span> {CATEGORY_LABELS[group.category] ?? group.category} ({group.items.length})
                            </span>
                            <span class="bud-group-total">{fmt(group.total)}</span>
                          </div>
                          <Show when={expanded().has(`g-${group.category}`)}>
                            <For each={group.items}>
                              {(it) => (
                                <div data-testid="budget-item-row" class="bud-item-row">
                                  <span class="bud-item-name">{it.name}</span>
                                  <span class="bud-item-meta">
                                    <strong>{fmt(it.tokens)}</strong> <span classList={{ 'bud-tag': true, measured: it.measured, estimated: !it.measured }}>{it.measured ? 'measured' : 'estimated'}</span>
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
                  <div data-testid="budget-empty" class="bud-empty">
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
    <div class="bud-shell">
      <aside class="bud-scopes">
        <div class="bud-eyebrow bud-scopes-title">Scope</div>
        <For each={props.scan.scopes}>
          {(s) => (
            <div
              data-testid="budget-scope-row"
              data-scope-id={s.id}
              onClick={() => setScopeId(s.id)}
              classList={{ 'bud-scope-row': true, active: scopeId() === s.id }}
            >
              {s.label}
            </div>
          )}
        </For>
      </aside>
      <main class="bud-main">
        <Show when={scope()} fallback={
          <div class="bud-pick-empty">Select a scope to view its context budget.</div>
        }>
          {(s) => <Budget scan={props.scan} scope={s()} />}
        </Show>
      </main>
    </div>
  );
}
