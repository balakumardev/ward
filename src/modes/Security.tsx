import '../styles/security.css';
import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import type { Finding, Severity, HarnessItem, RestoreInfo } from '../api';
import { api, isTauri } from '../api';

const SEVERITY_ORDER: Severity[] = ['critical', 'high', 'medium', 'low'];
/** severity → tint class (defined in security.css); sets --sev/--sev-bg. */
const SEV_CLASS: Record<Severity, string> = {
  critical: 'sec-sev-critical',
  high: 'sec-sev-high',
  medium: 'sec-sev-medium',
  low: 'sec-sev-low',
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

  // Plan 10 — `config-changed` is emitted by the Rust fs watcher
  // whenever `~/.claude` or `~/.codex` change on disk. Re-run the
  // security scan on each emission so the findings list reflects the
  // latest state without forcing the user to click "Scan now". The
  // listener lives for the component lifetime and is cleaned up on
  // unmount.
  let unlisten: (() => void) | null = null;
  listen<string[]>('config-changed', () => {
    void refetch();
  }).then((handle) => {
    unlisten = handle;
  });
  onCleanup(() => {
    if (unlisten) unlisten();
  });

  // Plan 10 — also listen for `scan-now` from the tray menu. Same
  // re-run path so the user sees immediate feedback after clicking
  // the tray's "Scan now" item.
  let unlistenScan: (() => void) | null = null;
  listen('scan-now', () => {
    void refetch();
  }).then((handle) => {
    unlistenScan = handle;
  });
  onCleanup(() => {
    if (unlistenScan) unlistenScan();
  });

  // Plan 15 — push each resolved scan's critical count to the native
  // dock badge + tray tooltip. Runs on every scan (initial + every
  // refetch). No-op outside the Tauri webview so dev:mock/jsdom don't
  // invoke a backend command that isn't there.
  createEffect(() => {
    const r = scan();
    if (r && isTauri()) {
      void api.nativeUpdateStatus(r.severityCounts.critical, r.timestamp);
    }
  });

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
    <div class="sec">
      {/* LEFT — findings grouped by severity */}
      <aside class="sec-list" data-testid="security-list">
        <div class="sec-list-top">
          <div class="sec-list-head">
            <div class="sec-list-title">Findings</div>
            <button class="btn btn-primary sec-scan" data-testid="security-scan-now" onClick={() => refetch()}>Scan now</button>
          </div>
          <Show when={scan()} fallback={<div class="sec-scanning">Scanning…</div>}>
            {(result) => (
              <div class="sec-counts" data-testid="security-counts">
                <span class="sec-count sec-sev-critical">● {result().severityCounts.critical}</span>
                <span class="sec-count sec-sev-high">● {result().severityCounts.high}</span>
                <span class="sec-count sec-sev-medium">● {result().severityCounts.medium}</span>
                <span class="sec-count sec-sev-low">● {result().severityCounts.low}</span>
              </div>
            )}
          </Show>
        </div>
        <div class="sec-findings">
          <For each={sortedFindings()}>
            {(f) => (
              <div
                data-testid="security-finding-row"
                data-rule-id={f.ruleId}
                classList={{ 'sec-finding': true, [SEV_CLASS[f.severity]]: true, selected: selected() === f.id }}
                onClick={() => setSelected(f.id)}
              >
                <span class="sec-sev-dot" data-testid="severity-dot" />
                <div class="sec-finding-main">
                  <div class="sec-finding-title">{f.ruleId} · {f.sourceName}</div>
                  <div class="sec-finding-sub">{f.matchedText}</div>
                </div>
              </div>
            )}
          </For>
          <Show when={scan() && sortedFindings().length === 0}>
            <div class="sec-clean" data-testid="security-no-findings">No findings — your MCP setup looks clean.</div>
          </Show>
        </div>
      </aside>

      {/* RIGHT — detail pane */}
      <main class="sec-detail" data-testid="security-detail">
        <Show when={selectedFinding()} fallback={<div class="sec-detail-empty">Select a finding to view details.</div>}>
          {(f) => (
            <div classList={{ 'sec-detail-card': true, rise: true, [SEV_CLASS[f().severity]]: true }}>
              <div class="sec-detail-head">
                <h2 class="sec-detail-title">{f().ruleId} — {f().name}</h2>
              </div>
              <div class="sec-detail-desc">{f().description}</div>
              <div class="sec-field" data-testid="security-source">
                <strong>Source:</strong> <code>{f().sourceName}</code>
              </div>
              <div class="sec-snippet" data-testid="security-snippet">{f().context}</div>
              <div class="sec-field" data-testid="security-matched">
                <strong>Matched:</strong> <code class="sec-matched-code">{f().matchedText}</code>
              </div>
              <div class="sec-actions">
                <button class="btn btn-ghost" data-testid="security-rejudge" onClick={() => rejudge()}>Re-judge</button>
                <button class="btn" data-testid="security-accept-baseline" onClick={() => acceptBaseline(f())}>Accept baseline</button>
              </div>
            </div>
          )}
        </Show>
      </main>
    </div>
  );
}
