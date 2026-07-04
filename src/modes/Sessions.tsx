import { createMemo, createSignal, For, Show } from 'solid-js';
import type {
  Conversation,
  CostBreakdown,
  DistillResult,
  HarnessItem,
  RestoreInfo,
  ScanResult,
  SessionRecord,
} from '../api';

/** Plan 07 — Sessions mode.
 *
 * Three-pane layout:
 *   LEFT   — list of session JSONL files (from `session` category in scan result).
 *   MIDDLE — read-only conversation viewer (User/Assistant turns with model + cost).
 *   RIGHT  — cost panel with per-model breakdown + Distill/Trim actions.
 *
 * Top toolbar buttons: Open (loads preview), Cost (recomputes), Distill,
 * Trim. Distill shows the resulting `index.md` inline. Trim emits an
 * Undo button that wires to the standard `RestoreInfo` pipeline.
 *
 * Sessions appear under category `session`; items can be file roots
 * (the JSONL itself) or children inside a bundle (e.g. backup/index
 * artifacts emitted by Distill).
 */

function fmtBytes(n: number): string {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}K`;
  return `${(n / (1024 * 1024)).toFixed(1)}M`;
}

function fmtTokens(n: number): string {
  return n.toLocaleString();
}

function fmtUsd(n: number): string {
  return `$${n.toFixed(3)}`;
}

function recordSummary(rec: SessionRecord): { label: string; sub: string } {
  switch (rec.kind) {
    case 'user':
      return {
        label: 'User',
        sub: rec.content.length > 140 ? rec.content.slice(0, 140) + '…' : rec.content,
      };
    case 'assistant': {
      const model = rec.model ? rec.model.replace(/^claude-/, '') : 'assistant';
      const txt = rec.content.length > 140 ? rec.content.slice(0, 140) + '…' : rec.content;
      return { label: `Assistant (${model})`, sub: txt };
    }
    case 'system':
      return { label: `System: ${rec.subtype}`, sub: rec.summary ?? '' };
    case 'aiTitle':
      return { label: 'Title', sub: rec.title };
    case 'queueOperation':
      return { label: 'Queue', sub: rec.enqueue ? 'enqueue' : 'dequeue' };
    case 'other':
      return { label: rec.recordType, sub: '' };
  }
}

function recordHasUsage(rec: SessionRecord): boolean {
  return rec.kind === 'assistant' && !!rec.usage;
}

export interface SessionsApi {
  sessionPreview: (path: string) => Promise<Conversation>;
  sessionCost: (path: string) => Promise<CostBreakdown>;
  sessionDistill: (path: string) => Promise<DistillResult>;
  sessionTrim: (path: string) => Promise<RestoreInfo>;
  restore: (info: RestoreInfo) => Promise<void>;
}

export function Sessions(props: { scan: ScanResult; api: SessionsApi }) {
  // Sessions are filtered to top-level JSONL files only. Bundle
  // children (backup/index artifacts from a prior Distill run) are
  // surfaced via `bundle` from the scan but we show them collapsed
  // here — only the parent session is meaningful for browsing.
  const sessionItems = createMemo<HarnessItem[]>(() =>
    props.scan.items.filter((i) => i.category === 'session')
  );

  const [selectedPath, setSelectedPath] = createSignal<string>(sessionItems()[0]?.path ?? '');
  const [conversation, setConversation] = createSignal<Conversation | null>(null);
  const [cost, setCost] = createSignal<CostBreakdown | null>(null);
  const [distillResult, setDistillResult] = createSignal<DistillResult | null>(null);
  const [trimInfo, setTrimInfo] = createSignal<RestoreInfo | null>(null);
  const [busy, setBusy] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  async function openSelected(path: string) {
    setSelectedPath(path);
    setConversation(null);
    setCost(null);
    setDistillResult(null);
    setTrimInfo(null);
    setError(null);
    setBusy('open');
    try {
      const c = await props.api.sessionPreview(path);
      setConversation(c);
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  }

  async function recomputeCost() {
    const path = selectedPath();
    if (!path) return;
    setBusy('cost');
    setError(null);
    try {
      const c = await props.api.sessionCost(path);
      setCost(c);
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  }

  async function distillSelected() {
    const path = selectedPath();
    if (!path) return;
    setBusy('distill');
    setError(null);
    try {
      const r = await props.api.sessionDistill(path);
      setDistillResult(r);
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  }

  async function trimSelected() {
    const path = selectedPath();
    if (!path) return;
    setBusy('trim');
    setError(null);
    try {
      const info = await props.api.sessionTrim(path);
      setTrimInfo(info);
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  }

  async function undoTrim() {
    const info = trimInfo();
    if (!info) return;
    setBusy('undo');
    setError(null);
    try {
      await props.api.restore(info);
      setTrimInfo(null);
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div data-testid="sessions-mode" style={{ display: 'flex', height: '100%', 'font-size': '12px' }}>
      {/* LEFT — session list */}
      <aside
        data-testid="sessions-list"
        style={{
          width: '260px', 'border-right': '1px solid var(--border)',
          padding: '8px', 'overflow-y': 'auto', background: 'var(--surface-2)',
        }}
      >
        <div style={{ 'font-size': '11px', color: 'var(--text-dim)', 'margin-bottom': '6px' }}>
          Sessions ({sessionItems().length})
        </div>
        <Show when={sessionItems().length > 0} fallback={
          <div data-testid="sessions-empty" style={{ color: 'var(--text-dim)', padding: '8px' }}>
            No session files found under <code>~/.claude/projects/</code>.
          </div>
        }>
          <For each={sessionItems()}>
            {(item) => (
              <div
                data-testid="sessions-row"
                data-path={item.path}
                onClick={() => openSelected(item.path)}
                style={{
                  padding: '6px 8px', margin: '2px 0', 'border-radius': 'var(--radius)',
                  cursor: 'pointer',
                  background: selectedPath() === item.path ? 'rgba(48,209,88,0.14)' : 'transparent',
                  color: selectedPath() === item.path ? 'var(--accent)' : 'var(--text)',
                }}
              >
                <div style={{ 'font-weight': 500, 'white-space': 'nowrap', overflow: 'hidden', 'text-overflow': 'ellipsis' }}>
                  {item.name}
                </div>
                <div style={{ 'font-size': '10px', color: 'var(--text-dim)', 'white-space': 'nowrap', overflow: 'hidden', 'text-overflow': 'ellipsis' }}>
                  {item.description || item.path.split('/').slice(-2).join('/')}
                </div>
              </div>
            )}
          </For>
        </Show>
      </aside>

      {/* MIDDLE — conversation viewer */}
      <main
        data-testid="sessions-viewer"
        style={{ flex: 1, 'overflow-y': 'auto', padding: '12px 16px' }}
      >
        <Show when={selectedPath()} fallback={
          <div style={{ color: 'var(--text-dim)' }}>Pick a session from the left.</div>
        }>
          {/* Toolbar */}
          <div style={{ display: 'flex', gap: '8px', 'margin-bottom': '10px', 'flex-wrap': 'wrap', 'align-items': 'center' }}>
            <button
              data-testid="sessions-btn-open"
              disabled={busy() !== null}
              onClick={() => openSelected(selectedPath())}
              style={btnStyle}
            >
              Open
            </button>
            <button
              data-testid="sessions-btn-cost"
              disabled={busy() !== null}
              onClick={recomputeCost}
              style={btnStyle}
            >
              Cost
            </button>
            <button
              data-testid="sessions-btn-distill"
              disabled={busy() !== null}
              onClick={distillSelected}
              style={{ ...btnStyle, background: 'var(--accent)', color: '#000' }}
            >
              Distill
            </button>
            <button
              data-testid="sessions-btn-trim"
              disabled={busy() !== null}
              onClick={trimSelected}
              style={{ ...btnStyle, background: '#ff9f0a', color: '#000' }}
            >
              Trim
            </button>
            <Show when={trimInfo()}>
              <button
                data-testid="sessions-btn-undo"
                disabled={busy() !== null}
                onClick={undoTrim}
                style={{ ...btnStyle, background: 'var(--crit)', color: '#fff' }}
              >
                Undo Trim
              </button>
            </Show>
            <Show when={busy()}>
              <span data-testid="sessions-busy" style={{ color: 'var(--text-dim)' }}>
                {busy()}…
              </span>
            </Show>
            <Show when={error()}>
              <span data-testid="sessions-error" style={{ color: 'var(--crit)' }}>
                {error()}
              </span>
            </Show>
          </div>

          <Show when={distillResult()} fallback={null}>
            {(r) => (
              <section
                data-testid="sessions-distill-result"
                style={{ 'margin-bottom': '12px', background: 'var(--surface-2)', padding: '10px', 'border-radius': 'var(--radius)' }}
              >
                <div style={{ 'font-weight': 600, 'margin-bottom': '6px' }}>
                  Distilled · {fmtBytes(r().originalBytes)} → {fmtBytes(r().cleanedBytes)}
                  {' '}<span style={{ color: 'var(--ok)' }}>(-{r().reductionPct.toFixed(1)}%)</span>
                </div>
                <div style={{ 'font-size': '11px', color: 'var(--text-dim)', 'margin-bottom': '8px' }}>
                  Backup: <code>{r().backupPath}</code><br />
                  Cleaned: <code>{r().cleanedPath}</code>
                </div>
                <pre
                  data-testid="sessions-distill-index"
                  style={{ 'white-space': 'pre-wrap', 'font-size': '11px', background: 'var(--surface)', padding: '8px', 'border-radius': 'var(--radius)', 'max-height': '240px', 'overflow-y': 'auto' }}
                >
                  {r().indexMd}
                </pre>
              </section>
            )}
          </Show>

          <Show when={conversation()} fallback={
            <Show when={!busy()} fallback={
              <div style={{ color: 'var(--text-dim)' }}>Loading conversation…</div>
            }>
              <div style={{ color: 'var(--text-dim)' }}>No conversation loaded. Click Open.</div>
            </Show>
          }>
            {(c) => (
              <div data-testid="sessions-records">
                <h2 style={{ 'font-size': '13px', 'margin': '0 0 8px' }}>
                  {c().sessionId} · {c().records.length} records
                </h2>
                <For each={c().records}>
                  {(rec, i) => {
                    const s = recordSummary(rec);
                    const isAssistant = rec.kind === 'assistant';
                    return (
                      <div
                        data-testid={`sessions-record-${i()}`}
                        data-kind={rec.kind}
                        style={{
                          'margin-bottom': '8px', padding: '8px 10px',
                          'border-left': '3px solid ' + (isAssistant ? 'var(--accent)' : 'var(--border)'),
                          background: 'var(--surface-2)', 'border-radius': 'var(--radius)',
                        }}
                      >
                        <div style={{ 'font-size': '10px', color: 'var(--text-dim)', 'margin-bottom': '2px' }}>
                          #{i() + 1} · {s.label}
                          <Show when={recordHasUsage(rec)}>
                            <span> · in={fmtTokens((rec as Extract<SessionRecord, { kind: 'assistant' }>).usage!.inputTokens)}
                              {' '}out={fmtTokens((rec as Extract<SessionRecord, { kind: 'assistant' }>).usage!.outputTokens)}</span>
                          </Show>
                        </div>
                        <div style={{ 'white-space': 'pre-wrap' }}>{s.sub || '(empty)'}</div>
                      </div>
                    );
                  }}
                </For>
              </div>
            )}
          </Show>
        </Show>
      </main>

      {/* RIGHT — cost panel */}
      <aside
        data-testid="sessions-cost-panel"
        style={{
          width: '320px', 'border-left': '1px solid var(--border)',
          padding: '12px', 'overflow-y': 'auto', background: 'var(--surface-2)',
        }}
      >
        <div style={{ 'font-size': '11px', color: 'var(--text-dim)', 'margin-bottom': '8px' }}>
          Cost breakdown
        </div>
        <Show when={cost()} fallback={
          <div data-testid="sessions-cost-empty" style={{ color: 'var(--text-dim)', 'font-size': '11px' }}>
            Click <strong>Cost</strong> to compute per-model token usage + estimated USD.
          </div>
        }>
          {(b) => (
            <div data-testid="sessions-cost-result">
              <div
                data-testid="sessions-cost-total"
                style={{ 'font-size': '16px', 'font-weight': 600, 'margin-bottom': '6px' }}
              >
                {fmtUsd(b().estimatedCostUsd)}
                <span style={{ 'font-size': '10px', color: 'var(--text-dim)', 'margin-left': '6px' }}>
                  estimated
                </span>
              </div>
              <Show when={b().estimatedRecords > 0}>
                <div style={{ 'font-size': '10px', color: '#ff9f0a', 'margin-bottom': '8px' }}>
                  ⚠ {b().estimatedRecords} record(s) used the fallback price
                </div>
              </Show>
              <table style={{ width: '100%', 'border-collapse': 'collapse', 'font-size': '11px' }}>
                <thead>
                  <tr style={{ color: 'var(--text-dim)', 'text-align': 'left' }}>
                    <th style={{ padding: '3px 6px' }}>Model</th>
                    <th style={{ padding: '3px 6px', 'text-align': 'right' }}>In</th>
                    <th style={{ padding: '3px 6px', 'text-align': 'right' }}>Out</th>
                    <th style={{ padding: '3px 6px', 'text-align': 'right' }}>$</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={b().perModel}>
                    {(row) => (
                      <tr data-testid="sessions-cost-row" data-model={row.model}>
                        <td style={{ padding: '3px 6px', 'white-space': 'nowrap', overflow: 'hidden', 'text-overflow': 'ellipsis', 'max-width': '140px' }}>
                          {row.model.replace(/^claude-/, '')}
                        </td>
                        <td style={{ padding: '3px 6px', 'text-align': 'right' }}>{fmtTokens(row.inputTokens)}</td>
                        <td style={{ padding: '3px 6px', 'text-align': 'right' }}>{fmtTokens(row.outputTokens)}</td>
                        <td style={{ padding: '3px 6px', 'text-align': 'right' }}>{fmtUsd(row.costUsd)}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
                <tfoot>
                  <tr style={{ 'border-top': '1px solid var(--border)' }}>
                    <td style={{ padding: '4px 6px', 'font-weight': 600 }}>Total</td>
                    <td style={{ padding: '4px 6px', 'text-align': 'right' }}>{fmtTokens(b().totalInputTokens)}</td>
                    <td style={{ padding: '4px 6px', 'text-align': 'right' }}>{fmtTokens(b().totalOutputTokens)}</td>
                    <td style={{ padding: '4px 6px', 'text-align': 'right' }}>{fmtUsd(b().estimatedCostUsd)}</td>
                  </tr>
                </tfoot>
              </table>
            </div>
          )}
        </Show>
      </aside>
    </div>
  );
}

const btnStyle = {
  padding: '4px 12px',
  'font-size': '11px',
  border: '1px solid var(--border)',
  background: 'var(--surface)',
  color: 'var(--text)',
  'border-radius': 'var(--radius)',
  cursor: 'pointer',
};