import { createMemo, createResource, createSignal, For, Show } from 'solid-js';
import type { BudgetItem, Scope, ScanResult } from '../api';
import { api } from '../api';
import '../styles/budget.css';

/** Plan 06 — Context Budget mode (real Claude Code consumption model).
 *
 * The hero meter fills toward `used` — the ALWAYS-ON total (system prompt
 * + built-in tools, ancestor CLAUDE.md, MEMORY.md, unscoped rules, the
 * capped skill/command + subagent listings, the MCP tool-names line, and
 * an active output style). This is what actually sits in every request's
 * context.
 *
 * A separate "On-invoke" panel shows the DEFERRED figure — skill/command/
 * agent bodies, MCP tool schemas, and `paths:`-scoped rules — which load
 * only when invoked and therefore do NOT count against the meter. Keeping
 * the two apart is the whole point: it stops skills (222K of bodies on the
 * real config) from being mis-counted as always-on.
 *
 * The user selects a scope from the rail; switching re-runs
 * `api.contextBudget` against the new scope_id.
 */

const CATEGORY_LABELS: Record<string, string> = {
  skill: 'Skill',
  rule: 'Rule',
  command: 'Command',
  agent: 'Agent',
  memory: 'Memory',
};

function pct(n: number, d: number): number {
  if (d <= 0) return 0;
  return Math.min(100, Math.round((n / d) * 1000) / 10);
}

function fmt(n: number): string {
  return n.toLocaleString();
}

/** Group budget items by category, sorted, tokens descending within. */
function groupByCategory(items: BudgetItem[]): Array<{ category: string; items: BudgetItem[]; total: number }> {
  const map = new Map<string, BudgetItem[]>();
  for (const it of items) {
    const arr = map.get(it.category) ?? [];
    arr.push(it);
    map.set(it.category, arr);
  }
  return [...map.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([category, its]) => ({
      category,
      items: its.slice().sort((a, b) => b.tokens - a.tokens),
      total: its.reduce((s, i) => s + i.tokens, 0),
    }));
}

export function Budget(props: { scan: ScanResult; scope: Scope }) {
  const [budget, { refetch }] = createResource(
    () => ({ harness: props.scan.harnessId, scopeId: props.scope.id }),
    ({ harness, scopeId }) => api.contextBudget(harness, scopeId),
  );

  // Per-group collapse state. Empty set = all collapsed.
  const [expanded, setExpanded] = createSignal<Set<string>>(new Set());
  function toggle(id: string) {
    const next = new Set(expanded());
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setExpanded(next);
  }

  const alwaysLoadedByCategory = createMemo(() => {
    const b = budget();
    return b ? groupByCategory(b.alwaysLoadedItems) : [];
  });
  const metadataByCategory = createMemo(() => {
    const b = budget();
    return b ? groupByCategory(b.metadataItems) : [];
  });
  const deferredByCategory = createMemo(() => {
    const b = budget();
    return b ? groupByCategory(b.deferredItems) : [];
  });

  return (
    <div data-testid="budget-mode" class="bud-mode">
      <header class="bud-header">
        <div>
          <h2 class="bud-title">Context Budget — {props.scope.label}</h2>
          <div data-testid="budget-method" class="bud-method">
            <Show when={budget()} fallback="Computing…">
              {(b) => (
                <span>
                  {b().measured ? 'measured (real tokenizer)' : 'estimated (bytes/4)'}
                  {' · '}{fmt(b().contextLimit)}-token context model
                </span>
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
          const fillState = fillColor === 'var(--crit)' ? 'crit'
            : fillColor === 'var(--ok)' ? 'ok'
            : 'warn';

          // ── Always-on component rows (these sum to `used`) ──
          const alwaysRows: Array<{ key: string; label: string; tokens: number }> = [
            { key: 'system', label: 'System prompt + built-in tools', tokens: b().systemLoaded },
          ];
          if (b().outputStyle > 0) {
            alwaysRows.push({ key: 'output-style', label: 'Output style', tokens: b().outputStyle });
          }
          alwaysRows.push({
            key: 'claudemd',
            label: `CLAUDE.md${b().claudeMdFiles.length > 1 ? ` (${b().claudeMdFiles.length} files)` : ''}`,
            tokens: b().claudemd,
          });
          const skillMeta = b().skillListing + b().skillBoilerplate;
          if (skillMeta > 0) {
            alwaysRows.push({ key: 'skill-listing', label: 'Skills + commands (listing)', tokens: skillMeta });
          }
          if (b().agentListing > 0) {
            alwaysRows.push({ key: 'agent-listing', label: 'Subagents (listing)', tokens: b().agentListing });
          }
          if (b().mcpToolNames > 0) {
            alwaysRows.push({ key: 'mcp-names', label: 'MCP tool names', tokens: b().mcpToolNames });
          }
          const always = alwaysLoadedByCategory();
          const metadata = metadataByCategory();
          const deferredGroups = deferredByCategory();

          // ── Deferred component rows (NOT counted in the meter) ──
          const deferredRows: Array<{ key: string; label: string; tokens: number }> = [
            { key: 'system-deferred', label: 'System tools (deferred via Tool Search)', tokens: b().systemDeferred },
          ];
          if (b().mcpSchemas > 0) {
            deferredRows.push({ key: 'mcp-schemas', label: 'MCP tool schemas', tokens: b().mcpSchemas });
          }
          const listingCapped = b().skillListingRaw > b().skillListing;

          return (
            <>
              {/* METER */}
              <section data-testid="budget-meter" class="bud-card bud-meter">
                <div class="bud-meter-top">
                  <div>
                    <div class="bud-eyebrow">Always-on · used</div>
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
                  <div class="bud-marker warn" title="warning threshold" style={{ left: `${pct(limit - b().warningThreshold, limit)}%` }} />
                  <div class="bud-marker crit" title="max output reservation" style={{ left: `${pct(limit - b().maxOutput, limit)}%` }} />
                </div>
                <div class="bud-legend">
                  <span><span class="glyph" style={{ color: '#ff9f0a' }}>▌</span> warn @ {fmt(warnAt)}</span>
                  <span><span class="glyph" style={{ color: 'var(--crit)' }}>▌</span> max-out @ {fmt(maxOutAt)}</span>
                  <span>autocompact @ {fmt(compactAt)}</span>
                  <span data-testid="budget-deferred-note" class="bud-legend-defer">+ {fmt(b().deferredTotal)} deferred (on-invoke, not counted)</span>
                </div>
              </section>

              {/* ALWAYS-ON BREAKDOWN */}
              <section data-testid="budget-breakdown" class="bud-card bud-breakdown">
                <h3 class="bud-section-title">Always-on — where the used tokens come from</h3>
                <div class="bud-bd-list">
                  <div class="bud-bd-headrow">
                    <span>Component</span>
                    <span class="bud-bd-head-num">Tokens</span>
                    <span class="bud-bd-head-share">Share</span>
                  </div>
                  <For each={alwaysRows}>
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
                    <span class="bud-bd-label">Total always-on (used)</span>
                    <span data-testid="budget-total" class="bud-bd-num">{fmt(total)}</span>
                    <span class="bud-bd-share"><span class="bud-bd-pct">100%</span></span>
                  </div>
                </div>
                <Show when={listingCapped}>
                  <div data-testid="budget-listing-capped" class="bud-note">
                    Skill/command listing capped to {fmt(b().skillListing)} (1% of context) — from {fmt(b().skillListingRaw)} raw.
                  </div>
                </Show>
              </section>

              {/* ON-INVOKE / DEFERRED */}
              <section data-testid="budget-deferred" class="bud-card bud-deferred">
                <h3 class="bud-section-title">On-invoke — deferred, loaded only when used</h3>
                <p class="bud-defer-lede">
                  These are <strong>not</strong> in the always-on total above. Skill/command/agent bodies,
                  MCP tool schemas, and <code>paths:</code>-scoped rules load on demand.
                </p>
                <div class="bud-bd-list">
                  <For each={deferredRows}>
                    {(row) => (
                      <div data-testid={`budget-row-deferred-${row.key}`} class="bud-bd-row">
                        <span class="bud-bd-label">{row.label}</span>
                        <span class="bud-bd-num">{fmt(row.tokens)}</span>
                        <span class="bud-bd-share">
                          <span class="bud-prop"><span class="bud-prop-fill defer" style={{ width: `${pct(row.tokens, b().deferredTotal)}%` }} /></span>
                          <span class="bud-bd-pct">{pct(row.tokens, b().deferredTotal)}%</span>
                        </span>
                      </div>
                    )}
                  </For>
                  <For each={deferredGroups}>
                    {(group) => (
                      <div data-testid={`budget-row-deferred-body-${group.category}`} class="bud-bd-row">
                        <span class="bud-bd-label">{CATEGORY_LABELS[group.category] ?? group.category} bodies ({group.items.length})</span>
                        <span class="bud-bd-num">{fmt(group.total)}</span>
                        <span class="bud-bd-share">
                          <span class="bud-prop"><span class="bud-prop-fill defer" style={{ width: `${pct(group.total, b().deferredTotal)}%` }} /></span>
                          <span class="bud-bd-pct">{pct(group.total, b().deferredTotal)}%</span>
                        </span>
                      </div>
                    )}
                  </For>
                  <div class="bud-bd-row bud-bd-total">
                    <span class="bud-bd-label">Total deferred</span>
                    <span data-testid="budget-deferred-total" class="bud-bd-num">{fmt(b().deferredTotal)}</span>
                    <span class="bud-bd-share"><span class="bud-bd-pct">100%</span></span>
                  </div>
                </div>
              </section>

              {/* DETAIL LIST */}
              <section data-testid="budget-detail" class="bud-card bud-detail">
                <h3 class="bud-section-title">Files &amp; items</h3>
                <Show when={b().claudeMdFiles.length > 0}>
                  <div class="bud-sub">
                    <div class="bud-sub-title">CLAUDE.md <span class="bud-sub-tag on">always-on</span></div>
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
                    <div class="bud-sub-title">Always-loaded items <span class="bud-sub-tag on">always-on</span></div>
                    <ItemGroups groups={always} prefix="al" expanded={expanded()} onToggle={toggle} />
                  </div>
                </Show>

                <Show when={metadata.length > 0}>
                  <div class="bud-sub">
                    <div class="bud-sub-title">Listing metadata <span class="bud-sub-tag on">always-on</span></div>
                    <ItemGroups groups={metadata} prefix="meta" expanded={expanded()} onToggle={toggle} />
                  </div>
                </Show>

                <Show when={deferredGroups.length > 0}>
                  <div class="bud-sub">
                    <div class="bud-sub-title">Deferred bodies &amp; scoped rules <span class="bud-sub-tag off">on-invoke</span></div>
                    <ItemGroups groups={deferredGroups} prefix="def" expanded={expanded()} onToggle={toggle} defer />
                  </div>
                </Show>

                <Show when={b().claudeMdFiles.length === 0 && always.length === 0 && metadata.length === 0 && deferredGroups.length === 0}>
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

/** Collapsible per-category item groups, shared by the always-on,
 *  metadata, and deferred detail sub-lists. */
function ItemGroups(props: {
  groups: Array<{ category: string; items: BudgetItem[]; total: number }>;
  prefix: string;
  expanded: Set<string>;
  onToggle: (id: string) => void;
  defer?: boolean;
}) {
  return (
    <For each={props.groups}>
      {(group) => {
        const id = `${props.prefix}-${group.category}`;
        return (
          <div data-testid={`budget-group-${props.prefix}-${group.category}`} class="bud-group">
            <div class="bud-group-head" onClick={() => props.onToggle(id)}>
              <span class="bud-group-name">
                <span class="bud-caret">{props.expanded.has(id) ? '▾' : '▸'}</span> {CATEGORY_LABELS[group.category] ?? group.category} ({group.items.length})
              </span>
              <span classList={{ 'bud-group-total': true, defer: !!props.defer }}>{fmt(group.total)}</span>
            </div>
            <Show when={props.expanded.has(id)}>
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
        );
      }}
    </For>
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
