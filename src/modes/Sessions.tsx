import { createMemo, createSignal, For, Show, type JSX } from 'solid-js';
import '../styles/sessions.css';
import type {
  ContentBlock,
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

/** Header label for a record row (role + model / subtype). */
function headLabel(rec: SessionRecord): string {
  switch (rec.kind) {
    case 'user':
      return 'User';
    case 'assistant': {
      const model = rec.model ? rec.model.replace(/^claude-/, '') : 'assistant';
      return `Assistant (${model})`;
    }
    case 'system':
      return `System: ${rec.subtype}`;
    case 'aiTitle':
      return 'Title';
    case 'summary':
      return 'Summary';
    case 'queueOperation':
      return 'Queue';
    case 'other':
      return rec.recordType;
  }
}

/** Body text for meta records (system/aiTitle/queue/other). Empty string
 *  means "render no body" — meta rows no longer show a misleading
 *  "(empty)" placeholder. */
function metaBody(rec: SessionRecord): string {
  switch (rec.kind) {
    case 'system':
      return rec.summary ?? '';
    case 'aiTitle':
      return rec.title;
    case 'queueOperation':
      return rec.enqueue ? 'enqueue' : 'dequeue';
    default:
      return '';
  }
}

function recordHasUsage(rec: SessionRecord): boolean {
  return rec.kind === 'assistant' && !!rec.usage;
}

/** Record kinds/types that render with no body — noise in the transcript.
 *  `other` records are always empty; `summary` is shown as the header title. */
function isNoiseRecord(rec: SessionRecord): boolean {
  return rec.kind === 'other' || rec.kind === 'summary';
}

/** Render a single structured content block as its own distinct row:
 *  normal text, a foldable dimmed `thinking` row, a `🔧 tool call` row,
 *  a `↳ result` row, or an image placeholder. */
function BlockRow(props: { block: ContentBlock }): JSX.Element {
  const b = props.block;
  switch (b.type) {
    case 'text':
      return (
        <div data-testid="sessions-block-text" class="sx-block sx-block--text">
          {b.text}
        </div>
      );
    case 'thinking':
      return (
        <details data-testid="sessions-block-thinking" class="sx-block sx-block--thinking">
          <summary class="sx-block-thinking-head">
            <span class="sx-block-tag">thinking</span>
          </summary>
          <div class="sx-block-thinking-body">{b.text}</div>
        </details>
      );
    case 'toolUse':
      return (
        <div data-testid="sessions-block-tooluse" class="sx-block sx-block--tooluse">
          <span class="sx-block-tool-icon" aria-hidden="true">🔧</span>
          <span class="sx-block-tool-name">{b.name}</span>
          <Show when={b.inputSummary}>
            <span class="sx-block-tool-input">{b.inputSummary}</span>
          </Show>
        </div>
      );
    case 'toolResult':
      return (
        <div data-testid="sessions-block-toolresult" class="sx-block sx-block--toolresult">
          <span class="sx-block-result-arrow" aria-hidden="true">↳</span>
          <span class="sx-block-result-body">{b.content}</span>
        </div>
      );
    case 'image':
      return (
        <div data-testid="sessions-block-image" class="sx-block sx-block--image">
          <span aria-hidden="true">🖼</span> <span>[image]</span>
        </div>
      );
  }
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
  const [showSystem, setShowSystem] = createSignal(false);

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
    <div data-testid="sessions-mode" class="sx-sessions">
      {/* LEFT — session list */}
      <aside data-testid="sessions-list" class="sx-list">
        <div class="sx-list-head">
          Sessions ({sessionItems().length})
        </div>
        <Show when={sessionItems().length > 0} fallback={
          <div data-testid="sessions-empty" class="sx-empty">
            No session files found under <code>~/.claude/projects/</code>.
          </div>
        }>
          <For each={sessionItems()}>
            {(item) => (
              <div
                data-testid="sessions-row"
                data-path={item.path}
                onClick={() => openSelected(item.path)}
                classList={{ 'sx-row': true, 'is-selected': selectedPath() === item.path }}
              >
                <div class="sx-row-name">
                  {item.name}
                </div>
                <div class="sx-row-sub">
                  {item.description || item.path.split('/').slice(-2).join('/')}
                </div>
              </div>
            )}
          </For>
        </Show>
      </aside>

      {/* MIDDLE — conversation viewer */}
      <main data-testid="sessions-viewer" class="sx-viewer">
        <Show when={selectedPath()} fallback={
          <div class="sx-hint">Pick a session from the left.</div>
        }>
          {/* Toolbar */}
          <div class="sx-toolbar">
            <button
              data-testid="sessions-btn-open"
              class="btn btn-ghost"
              disabled={busy() !== null}
              onClick={() => openSelected(selectedPath())}
            >
              Open
            </button>
            <button
              data-testid="sessions-btn-cost"
              class="btn btn-ghost"
              disabled={busy() !== null}
              onClick={recomputeCost}
            >
              Cost
            </button>
            <button
              data-testid="sessions-btn-distill"
              class="btn btn-primary"
              disabled={busy() !== null}
              onClick={distillSelected}
            >
              Distill
            </button>
            <button
              data-testid="sessions-btn-trim"
              class="btn sx-btn-warn"
              disabled={busy() !== null}
              onClick={trimSelected}
            >
              Trim
            </button>
            <Show when={trimInfo()}>
              <button
                data-testid="sessions-btn-undo"
                class="btn btn-danger"
                disabled={busy() !== null}
                onClick={undoTrim}
              >
                Undo Trim
              </button>
            </Show>
            <Show when={busy()}>
              <span data-testid="sessions-busy" class="sx-busy">
                {busy()}…
              </span>
            </Show>
            <Show when={error()}>
              <span data-testid="sessions-error" class="sx-error">
                {error()}
              </span>
            </Show>
          </div>

          <Show when={distillResult()} fallback={null}>
            {(r) => (
              <section data-testid="sessions-distill-result" class="sx-distill">
                <div class="sx-distill-title">
                  Distilled · {fmtBytes(r().originalBytes)} → {fmtBytes(r().cleanedBytes)}
                  {' '}<span class="sx-reduction">(-{r().reductionPct.toFixed(1)}%)</span>
                </div>
                <div class="sx-distill-meta">
                  Backup: <code>{r().backupPath}</code><br />
                  Cleaned: <code>{r().cleanedPath}</code>
                </div>
                <pre data-testid="sessions-distill-index" class="sx-distill-index">
                  {r().indexMd}
                </pre>
              </section>
            )}
          </Show>

          <Show when={conversation()} fallback={
            <Show when={!busy()} fallback={
              <div class="sx-hint">Loading conversation…</div>
            }>
              <div class="sx-hint">No conversation loaded. Click Open.</div>
            </Show>
          }>
            {(c) => (
              <div data-testid="sessions-records" class="sx-records">
                <header class="sx-convo-head">
                  <div class="sx-convo-title">{c().title || c().sessionId}</div>
                  <div class="sx-convo-meta">
                    <span>{c().records.length} records</span>
                    <button
                      type="button"
                      class="sx-convo-path"
                      title="Copy path"
                      onClick={() => navigator.clipboard?.writeText(selectedPath())}
                      data-testid="sessions-path"
                    >
                      {selectedPath()}
                    </button>
                  </div>
                </header>
                <Show when={c().records.filter(isNoiseRecord).length > 0}>
                  <button
                    type="button"
                    class="sx-system-toggle"
                    data-testid="sessions-toggle-system"
                    onClick={() => setShowSystem((v) => !v)}
                  >
                    {showSystem() ? 'Hide' : 'Show'} {c().records.filter(isNoiseRecord).length} system events
                  </button>
                </Show>
                <For each={c().records}>
                  {(rec, i) => {
                    const isAssistant = rec.kind === 'assistant';
                    const isUser = rec.kind === 'user';
                    const isMeta = !isAssistant && !isUser;
                    const blocks = () =>
                      (rec as Extract<SessionRecord, { kind: 'user' | 'assistant' }>).blocks ?? [];
                    return (
                      <Show when={showSystem() || !isNoiseRecord(rec)}>
                      <div
                        data-testid={`sessions-record-${i()}`}
                        data-kind={rec.kind}
                        classList={{
                          'sx-msg': true,
                          'sx-msg--assistant': isAssistant,
                          'sx-msg--user': isUser,
                          'sx-msg--meta': isMeta,
                        }}
                      >
                        <div class="sx-msg-head">
                          <span class="sx-msg-idx">#{i() + 1}</span>{' · '}<span class="sx-msg-role">{headLabel(rec)}</span>
                          <Show when={recordHasUsage(rec)}>
                            <span class="sx-msg-usage"> · in={fmtTokens((rec as Extract<SessionRecord, { kind: 'assistant' }>).usage!.inputTokens)}
                              {' '}out={fmtTokens((rec as Extract<SessionRecord, { kind: 'assistant' }>).usage!.outputTokens)}</span>
                          </Show>
                        </div>
                        <Show
                          when={!isMeta}
                          fallback={
                            <Show when={metaBody(rec)}>
                              <div class="sx-msg-body">{metaBody(rec)}</div>
                            </Show>
                          }
                        >
                          <div class="sx-msg-blocks" data-testid="sessions-msg-blocks">
                            <For each={blocks()}>
                              {(block) => <BlockRow block={block} />}
                            </For>
                            <Show when={blocks().length === 0}>
                              <div data-testid="sessions-block-empty" class="sx-msg-body sx-block-empty">
                                (no content)
                              </div>
                            </Show>
                          </div>
                        </Show>
                      </div>
                      </Show>
                    );
                  }}
                </For>
              </div>
            )}
          </Show>
        </Show>
      </main>

      {/* RIGHT — cost panel */}
      <aside data-testid="sessions-cost-panel" class="sx-cost">
        <div class="sx-cost-head">
          Cost breakdown
        </div>
        <Show when={cost()} fallback={
          <div data-testid="sessions-cost-empty" class="sx-cost-empty">
            Click <strong>Cost</strong> to compute per-model token usage + estimated USD.
          </div>
        }>
          {(b) => (
            <div data-testid="sessions-cost-result" class="sx-in">
              <div data-testid="sessions-cost-total" class="sx-cost-total">
                {fmtUsd(b().estimatedCostUsd)}
                <span class="sx-cost-est-label">
                  estimated
                </span>
              </div>
              <Show when={b().estimatedRecords > 0}>
                <div class="sx-cost-warn">
                  ⚠ {b().estimatedRecords} record(s) used the fallback price
                </div>
              </Show>
              <div class="sx-cost-card">
                <table class="sx-cost-table">
                  <thead>
                    <tr>
                      <th>Model</th>
                      <th>In</th>
                      <th>Out</th>
                      <th>$</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={b().perModel}>
                      {(row) => (
                        <tr data-testid="sessions-cost-row" data-model={row.model}>
                          <td>
                            {row.model.replace(/^claude-/, '')}
                          </td>
                          <td>{fmtTokens(row.inputTokens)}</td>
                          <td>{fmtTokens(row.outputTokens)}</td>
                          <td class="sx-cost-usd">{fmtUsd(row.costUsd)}</td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                  <tfoot>
                    <tr>
                      <td>Total</td>
                      <td>{fmtTokens(b().totalInputTokens)}</td>
                      <td>{fmtTokens(b().totalOutputTokens)}</td>
                      <td class="sx-cost-usd">{fmtUsd(b().estimatedCostUsd)}</td>
                    </tr>
                  </tfoot>
                </table>
              </div>
            </div>
          )}
        </Show>
      </aside>
    </div>
  );
}