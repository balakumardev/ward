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

/** Plain-English "what this severity means" line, so the number in the header
 *  isn't just a colour. */
const SEV_MEANING: Record<Severity, string> = {
  critical: 'Critical — very likely malicious or destructive. Investigate before running this server.',
  high: 'High — strong indicator of unsafe behaviour. Review carefully.',
  medium: 'Medium — worth a look; may be a false positive in benign text.',
  low: 'Low — informational; usually safe but flagged for awareness.',
};

/** Remediation guidance keyed by the rule-id prefix (rule family). Each MCP
 *  finding comes from one of these families; this is the "what do I do about
 *  it" the raw scanner output was missing. Kept on the frontend so we can
 *  tune the wording without a backend rebuild. */
const REMEDIATION: Record<string, { label: string; advice: string }> = {
  PI: {
    label: 'Prompt injection',
    advice: 'The server text tries to override the agent\'s instructions. If you don\'t recognise/trust this server, disable it in the Organizer or remove it. Legitimate servers rarely embed instruction-override language in their descriptions.',
  },
  TP: {
    label: 'Tool poisoning',
    advice: 'The tool description hints at hidden, secondary actions (collecting or sending data beyond its stated job). Verify the source repo and the tool\'s actual behaviour before trusting it; disable it if you can\'t.',
  },
  TS: {
    label: 'Tool shadowing',
    advice: 'This tool tries to influence how other tools behave. Prefer servers that stay in their own lane; remove or disable it if the cross-tool language isn\'t clearly justified.',
  },
  SF: {
    label: 'Sensitive file access',
    advice: 'The config references sensitive paths (SSH keys, credentials, .env, system files). Confirm the server genuinely needs this access. Scope it down or disable it if not.',
  },
  DE: {
    label: 'Data exfiltration',
    advice: 'The config points at an external/known-exfiltration endpoint. Confirm the destination is one you trust. If it\'s a tunneling/collector service (webhook.site, ngrok, etc.) treat it as hostile.',
  },
  CH: {
    label: 'Credential exposure',
    advice: 'A secret (API key, token, private key) appears in plaintext in the config. Rotate it immediately, move it to an environment variable, and remove the literal from the file.',
  },
  CE: {
    label: 'Code execution',
    advice: 'The config contains dynamic code-execution patterns (eval/exec, reverse shell, curl|bash). Only keep this if you fully trust and understand the command; otherwise disable the server.',
  },
  CI: {
    label: 'Command injection',
    advice: 'A destructive or network-exfil shell command is present. Review the exact command; remove or sandbox it if it isn\'t clearly safe.',
  },
  HK: {
    label: 'Suspicious hook',
    advice: 'A hook runs risky shell behaviour (downloads-and-executes, destructive ops, injected variables). Review the hook command in your settings.json and remove it if you didn\'t add it intentionally.',
  },
  EP: {
    label: 'Exfiltration parameter',
    advice: 'A tool parameter name suggests a hidden data channel (e.g. "note", "debug", "callback_url"). Check what the tool does with it; disable the server if the channel isn\'t legitimate.',
  },
  SC: {
    label: 'Supply chain',
    advice: 'The command auto-installs packages without confirmation (npx -y). Pin a specific version and review the package before allowing it to run unattended.',
  },
  PE: {
    label: 'Persistence',
    advice: 'The config modifies shell profiles or installs scheduled tasks/services — classic persistence. Confirm this is intentional; remove it otherwise.',
  },
};

/** Look up remediation by the rule-id family (prefix before the dash). */
function remediationFor(ruleId: string): { label: string; advice: string } | null {
  const prefix = (ruleId.split('-')[0] ?? '').toUpperCase();
  return REMEDIATION[prefix] ?? null;
}

/** Split the finding context around the matched text so the pane can highlight
 *  exactly what tripped the rule. Falls back to [before=context, ''] when the
 *  match isn't found inside the context string. */
function splitContext(context: string, matched: string): [string, string, string] {
  if (!matched) return [context, '', ''];
  const i = context.indexOf(matched);
  if (i < 0) return [context, '', ''];
  return [context.slice(0, i), matched, context.slice(i + matched.length)];
}

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
                  <div class="sec-finding-title">{f.name}</div>
                  <div class="sec-finding-sub">
                    <span class="sec-finding-source">{f.sourceName || 'unknown source'}</span>
                    <span class="sec-finding-rule">{f.ruleId}</span>
                  </div>
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
          {(f) => {
            const rem = () => remediationFor(f().ruleId);
            const parts = () => splitContext(f().context, f().matchedText);
            return (
            <div classList={{ 'sec-detail-card': true, rise: true, [SEV_CLASS[f().severity]]: true }}>
              <div class="sec-detail-head">
                <div class="sec-detail-headrow">
                  <span class="sec-sev-badge" data-testid="security-sev-badge">{f().severity}</span>
                  <h2 class="sec-detail-title">{f().name}</h2>
                </div>
                <div class="sec-detail-tags">
                  <Show when={rem()}><span class="sec-tag">{rem()!.label}</span></Show>
                  <span class="sec-tag sec-tag-dim">{f().ruleId}</span>
                </div>
              </div>

              {/* What it is */}
              <div class="sec-section">
                <div class="sec-section-label">What this means</div>
                <div class="sec-detail-desc">{f().description}</div>
                <div class="sec-sev-meaning">{SEV_MEANING[f().severity]}</div>
              </div>

              {/* Where it is */}
              <div class="sec-section">
                <div class="sec-section-label">Where</div>
                <div class="sec-field" data-testid="security-source">
                  <span class="sec-field-key">Source</span>
                  <code>{f().sourceName || 'unknown'}</code>
                </div>
                <div class="sec-snippet" data-testid="security-snippet">
                  <Show when={parts()[1]} fallback={<span>{f().context}</span>}>
                    <span>{parts()[0]}</span>
                    <mark class="sec-snippet-hit" data-testid="security-matched">{parts()[1]}</mark>
                    <span>{parts()[2]}</span>
                  </Show>
                </div>
              </div>

              {/* What to do */}
              <Show when={rem()}>
                <div class="sec-section sec-section-fix">
                  <div class="sec-section-label">What to do</div>
                  <div class="sec-fix-advice" data-testid="security-remediation">{rem()!.advice}</div>
                </div>
              </Show>

              <div class="sec-actions">
                <button class="btn btn-ghost" data-testid="security-rejudge" onClick={() => rejudge()}>Re-scan</button>
                <button class="btn" data-testid="security-accept-baseline" onClick={() => acceptBaseline(f())}>Mark as reviewed</button>
              </div>
            </div>
            );
          }}
        </Show>
      </main>
    </div>
  );
}
