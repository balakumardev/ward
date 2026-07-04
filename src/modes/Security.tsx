import { createMemo, createResource, createSignal, For, Show } from 'solid-js';
import type { Finding, Severity, HarnessItem, RestoreInfo } from '../api';
import { api } from '../api';

const SEVERITY_ORDER: Severity[] = ['critical', 'high', 'medium', 'low'];
const SEVERITY_COLORS: Record<Severity, string> = {
  critical: 'var(--danger)',
  high: '#ff8c42',
  medium: 'var(--warning)',
  low: 'var(--text-dim)',
};

export interface SecurityApi {
  listDestinations: (item: HarnessItem) => Promise<unknown[]>;
  moveItem: (item: HarnessItem, destScopeId: string) => Promise<RestoreInfo>;
  deleteItem: (item: HarnessItem) => Promise<RestoreInfo>;
  restore: (info: RestoreInfo) => Promise<void>;
  mcpSetDisabled: (projectPath: string, list: string[]) => Promise<RestoreInfo>;
}

function severityRank(s: Severity): number { return SEVERITY_ORDER.indexOf(s); }

/** Plan 05 — master-detail security findings UI. */
export function Security(props: {
  items: HarnessItem[];
  api: SecurityApi;
}) {
  const [scan, { refetch }] = createResource(() => api.securityScan('claude', props.items));
  const [selected, setSelected] = createSignal<string | null>(null);

  const sortedFindings = createMemo(() => {
    const result = scan();
    if (!result) return [] as Finding[];
    return [...result.findings].sort((a, b) => severityRank(a.severity) - severityRank(b.severity));
  });

  function getFinding(id: string): Finding | undefined {
    return sortedFindings().find((f) => f.id === id);
  }

  const selectedFinding = createMemo(() => {
    const id = selected();
    if (!id) return null;
    return getFinding(id) ?? null;
  });

  async function rejudge() {
    // Placeholder: would call api.securityScan with runJudge=true.
    await refetch();
  }

  async function acceptBaseline(f: Finding) {
    await api.securityBaselineAccept(f.sourceName, [f.ruleId]);
  }

  return (
    <div style={{ display: 'flex', height: '100%', 'font-size': '12px' }}>
      {/* LEFT — findings grouped by severity */}
      <aside data-testid="security-list"
        style={{ width: '340px', 'border-right': '1px solid var(--border)', 'overflow-y': 'auto', padding: '8px' }}>
        <div style={{ display: 'flex', 'align-items': 'center', 'justify-content': 'space-between', 'margin-bottom': '8px' }}>
          <div style={{ 'font-weight': 600 }}>Findings</div>
          <button data-testid="security-scan-now" onClick={() => refetch()}
            style={{ padding: '2px 8px', 'font-size': '11px' }}>Scan now</button>
        </div>
        <Show when={scan()} fallback={<div>Scanning…</div>}>
          {(result) => (
            <div data-testid="security-counts" style={{ display: 'flex', gap: '8px', 'font-size': '11px', 'margin-bottom': '8px', 'flex-wrap': 'wrap' }}>
              <span style={{ color: SEVERITY_COLORS.critical }}>● {result().severityCounts.critical}</span>
              <span style={{ color: SEVERITY_COLORS.high }}>● {result().severityCounts.high}</span>
              <span style={{ color: SEVERITY_COLORS.medium }}>● {result().severityCounts.medium}</span>
              <span style={{ color: SEVERITY_COLORS.low }}>● {result().severityCounts.low}</span>
            </div>
          )}
        </Show>
        <For each={sortedFindings()}>
          {(f) => (
            <div
              data-testid="security-finding-row"
              data-rule-id={f.ruleId}
              onClick={() => setSelected(f.id)}
              style={{
                padding: '6px 8px',
                'border-radius': 'var(--radius)',
                cursor: 'pointer',
                'margin-bottom': '4px',
                background: selected() === f.id ? 'rgba(48,209,88,0.14)' : 'transparent',
                display: 'flex', gap: '6px', 'align-items': 'flex-start',
              }}
            >
              <span data-testid="severity-dot" style={{ 'min-width': '8px', 'min-height': '8px', 'border-radius': '50%', background: SEVERITY_COLORS[f.severity], 'margin-top': '4px' }} />
              <div style={{ flex: 1, 'min-width': 0 }}>
                <div style={{ 'font-weight': 500, 'white-space': 'nowrap', overflow: 'hidden', 'text-overflow': 'ellipsis' }}>
                  {f.ruleId} · {f.sourceName}
                </div>
                <div style={{ color: 'var(--text-dim)', 'font-size': '11px', 'white-space': 'nowrap', overflow: 'hidden', 'text-overflow': 'ellipsis' }}>
                  {f.matchedText}
                </div>
              </div>
            </div>
          )}
        </For>
        <Show when={scan() && sortedFindings().length === 0}>
          <div data-testid="security-no-findings" style={{ color: 'var(--accent)', padding: '8px' }}>No findings — your MCP setup looks clean.</div>
        </Show>
      </aside>

      {/* RIGHT — detail pane */}
      <main data-testid="security-detail" style={{ flex: 1, padding: '12px', 'overflow-y': 'auto' }}>
        <Show when={selectedFinding()} fallback={<div style={{ color: 'var(--text-dim)' }}>Select a finding to view details.</div>}>
          {(f) => (
            <div>
              <h2 style={{ 'margin-top': 0 }}>{f().ruleId} — {f().name}</h2>
              <div style={{ color: 'var(--text-dim)', 'margin-bottom': '8px' }}>{f().description}</div>
              <div data-testid="security-source" style={{ 'font-size': '11px', 'margin-bottom': '12px' }}>
                <strong>Source:</strong> <code>{f().sourceName}</code>
              </div>
              <div data-testid="security-snippet" style={{
                padding: '8px', background: 'var(--surface-2)', 'border-radius': 'var(--radius)',
                'font-family': 'monospace', 'white-space': 'pre-wrap', 'word-break': 'break-word',
                'margin-bottom': '12px',
              }}>{f().context}</div>
              <div data-testid="security-matched" style={{ 'margin-bottom': '12px' }}>
                <strong>Matched:</strong> <code style={{ color: SEVERITY_COLORS[f().severity] }}>{f().matchedText}</code>
              </div>
              <div style={{ display: 'flex', gap: '6px', 'flex-wrap': 'wrap' }}>
                <button data-testid="security-rejudge" onClick={() => rejudge()} style={{ padding: '4px 10px', 'font-size': '11px' }}>Re-judge</button>
                <button data-testid="security-accept-baseline" onClick={() => acceptBaseline(f())} style={{ padding: '4px 10px', 'font-size': '11px' }}>Accept baseline</button>
              </div>
            </div>
          )}
        </Show>
      </main>
    </div>
  );
}
