import { createMemo, createResource, createSignal, Show } from 'solid-js';
import type {
  BackupStatus,
  CommitInfo,
  ExportReport,
  PushResult,
  ScanResult,
} from '../api';

export interface BackupsApi {
  backupStatus: () => Promise<BackupStatus>;
  backupRun: (scan: ScanResult, remoteUrl?: string | null) => Promise<ExportReport>;
  backupSync: () => Promise<CommitInfo>;
  backupPush: () => Promise<PushResult>;
  backupSchedulerInstall: (intervalSeconds: number) => Promise<void>;
  backupSchedulerRemove: () => Promise<void>;
  backupSetRemote: (url: string) => Promise<void>;
}

const MIN_INTERVAL_SECONDS = 300;
const MAX_INTERVAL_SECONDS = 86_400;

function fmtDate(iso: string | null): string {
  if (!iso) return '—';
  try {
    const d = new Date(iso);
    return d.toLocaleString();
  } catch {
    return iso;
  }
}

function shortSha(sha: string | null): string {
  if (!sha) return '—';
  return sha.length > 10 ? sha.slice(0, 10) : sha;
}

export function Backups(props: { scan: ScanResult; api: BackupsApi }) {
  const [status, { refetch: refetchStatus }] = createResource(() => props.api.backupStatus());

  const [busy, setBusy] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [info, setInfo] = createSignal<string | null>(null);

  // Remote URL form
  const [remoteDraft, setRemoteDraft] = createSignal<string>('');
  // Scheduler interval form
  const [intervalDraft, setIntervalDraft] = createSignal<number>(3600);

  async function runBackup() {
    setBusy('run');
    setError(null);
    setInfo(null);
    try {
      const r = await props.api.backupRun(props.scan, null);
      setInfo(`Exported ${r.filesCopied} file(s), ${r.bytesCopied} bytes to ~/.ward-backups/.`);
      await refetchStatus();
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  }

  async function syncBackup() {
    setBusy('sync');
    setError(null);
    setInfo(null);
    try {
      const c = await props.api.backupSync();
      if (c.committed) {
        setInfo(`Committed: ${shortSha(c.sha)} — ${c.message}`);
      } else {
        setInfo('Nothing to commit (working tree clean).');
      }
      await refetchStatus();
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  }

  async function pushBackup() {
    setBusy('push');
    setError(null);
    setInfo(null);
    try {
      const r = await props.api.backupPush();
      if (r.pushed) {
        setInfo(`Pushed to ${r.remoteUrl}.`);
      } else {
        setInfo(`Not pushed: ${r.reason}`);
      }
      await refetchStatus();
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  }

  async function setRemote() {
    const url = remoteDraft().trim();
    if (!url) {
      setError('Remote URL is required.');
      return;
    }
    setBusy('remote');
    setError(null);
    try {
      await props.api.backupSetRemote(url);
      setInfo(`Remote origin -> ${url}`);
      setRemoteDraft('');
      await refetchStatus();
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  }

  async function installScheduler() {
    const secs = intervalDraft();
    if (secs < MIN_INTERVAL_SECONDS || secs > MAX_INTERVAL_SECONDS) {
      setError(
        `Interval must be between ${MIN_INTERVAL_SECONDS}s and ${MAX_INTERVAL_SECONDS}s.`,
      );
      return;
    }
    setBusy('install');
    setError(null);
    try {
      await props.api.backupSchedulerInstall(secs);
      setInfo(`Installed launchd backup agent (interval=${secs}s).`);
      await refetchStatus();
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  }

  async function removeScheduler() {
    setBusy('remove');
    setError(null);
    try {
      await props.api.backupSchedulerRemove();
      setInfo('Removed launchd backup agent.');
      await refetchStatus();
    } catch (e) {
      setError(String((e as Error).message ?? e));
    } finally {
      setBusy(null);
    }
  }

  const scheduledInterval = createMemo(() => status()?.schedulerInterval ?? null);

  return (
    <div data-testid="backups-mode" style={{ padding: '16px', 'font-size': '12px' }}>
      <h2 style={{ 'font-size': '14px', margin: '0 0 12px' }}>Backups</h2>

      <Show when={status()} fallback={
        <div data-testid="backups-loading" style={{ color: 'var(--text-dim)' }}>Loading backup status…</div>
      }>
        {(s) => (
          <div style={{ display: 'grid', gap: '14px', 'grid-template-columns': '1fr', 'max-width': '780px' }}>
            {/* Status panel */}
            <section
              data-testid="backups-status-panel"
              style={{ background: 'var(--surface-2)', padding: '12px', 'border-radius': 'var(--radius)' }}
            >
              <div style={{ 'font-weight': 600, 'margin-bottom': '6px' }}>Status</div>
              <table style={{ width: '100%', 'border-collapse': 'collapse' }}>
                <tbody>
                  <tr>
                    <td style={cellLabel}>Repo</td>
                    <td style={cellValue}>
                      {s().hasRepo
                        ? <span data-testid="backups-repo-present" style={{ color: 'var(--ok)' }}>present (~/.ward-backups/)</span>
                        : <span data-testid="backups-repo-missing" style={{ color: 'var(--warn)' }}>not yet initialized</span>}
                    </td>
                  </tr>
                  <tr>
                    <td style={cellLabel}>Last commit</td>
                    <td style={cellValue}>
                      <span data-testid="backups-last-commit">{shortSha(s().lastCommit)}</span>
                      {' · '}
                      <span style={{ color: 'var(--text-dim)' }}>{fmtDate(s().lastCommitAt)}</span>
                    </td>
                  </tr>
                  <tr>
                    <td style={cellLabel}>Remote</td>
                    <td style={cellValue}>
                      <Show when={s().remoteUrl} fallback={<span style={{ color: 'var(--text-dim)' }}>none</span>}>
                        {(u) => <code data-testid="backups-remote">{u()}</code>}
                      </Show>
                    </td>
                  </tr>
                  <tr>
                    <td style={cellLabel}>Scheduler</td>
                    <td style={cellValue}>
                      <Show when={s().schedulerInstalled} fallback={
                        <span data-testid="backups-scheduler-not-installed" style={{ color: 'var(--text-dim)' }}>
                          not installed
                        </span>
                      }>
                        <span data-testid="backups-scheduler-installed" style={{ color: 'var(--ok)' }}>
                          installed · {s().schedulerInterval}s
                        </span>
                      </Show>
                    </td>
                  </tr>
                </tbody>
              </table>
            </section>

            {/* Run / Sync / Push */}
            <section style={{ background: 'var(--surface-2)', padding: '12px', 'border-radius': 'var(--radius)' }}>
              <div style={{ 'font-weight': 600, 'margin-bottom': '8px' }}>Manual backup</div>
              <div style={{ display: 'flex', gap: '8px', 'flex-wrap': 'wrap' }}>
                <button data-testid="backups-btn-run" disabled={busy() !== null} onClick={runBackup} style={btnPrimary}>
                  Run backup
                </button>
                <button data-testid="backups-btn-sync" disabled={busy() !== null} onClick={syncBackup} style={btnStyle}>
                  Sync (commit)
                </button>
                <button
                  data-testid="backups-btn-push"
                  disabled={busy() !== null}
                  onClick={pushBackup}
                  style={{ ...btnStyle, background: 'var(--accent)', color: '#000' }}
                >
                  Push
                </button>
              </div>
              <div style={{ 'font-size': '10px', color: 'var(--text-dim)', 'margin-top': '6px' }}>
                Push is the only network action — requires an explicit click.
              </div>
            </section>

            {/* Remote URL */}
            <section style={{ background: 'var(--surface-2)', padding: '12px', 'border-radius': 'var(--radius)' }}>
              <div style={{ 'font-weight': 600, 'margin-bottom': '8px' }}>Remote</div>
              <div style={{ display: 'flex', gap: '8px', 'flex-wrap': 'wrap', 'align-items': 'center' }}>
                <input
                  data-testid="backups-remote-input"
                  type="text"
                  placeholder="git@github.com:you/ward-backups.git"
                  value={remoteDraft()}
                  onInput={(e) => setRemoteDraft(e.currentTarget.value)}
                  style={inputStyle}
                />
                <button data-testid="backups-remote-set" disabled={busy() !== null} onClick={setRemote} style={btnStyle}>
                  Set remote
                </button>
              </div>
              <Show when={s().remoteUrl}>
                <div style={{ 'font-size': '10px', color: 'var(--text-dim)', 'margin-top': '6px' }}>
                  Current: <code>{s().remoteUrl}</code>
                </div>
              </Show>
            </section>

            {/* Scheduler */}
            <section
              data-testid="backups-scheduler-panel"
              style={{ background: 'var(--surface-2)', padding: '12px', 'border-radius': 'var(--radius)' }}
            >
              <div style={{ 'font-weight': 600, 'margin-bottom': '8px' }}>Scheduler (launchd)</div>
              <div style={{ display: 'flex', gap: '8px', 'flex-wrap': 'wrap', 'align-items': 'center' }}>
                <label style={{ display: 'flex', 'align-items': 'center', gap: '6px' }}>
                  Interval (sec):
                  <input
                    data-testid="backups-interval-input"
                    type="number"
                    min={MIN_INTERVAL_SECONDS}
                    max={MAX_INTERVAL_SECONDS}
                    value={intervalDraft()}
                    onInput={(e) => setIntervalDraft(Number(e.currentTarget.value) || MIN_INTERVAL_SECONDS)}
                    style={{ ...inputStyle, width: '110px' }}
                  />
                </label>
                <button
                  data-testid="backups-scheduler-install"
                  disabled={busy() !== null}
                  onClick={installScheduler}
                  style={btnStyle}
                >
                  Install
                </button>
                <button
                  data-testid="backups-scheduler-remove"
                  disabled={busy() !== null || !s().schedulerInstalled}
                  onClick={removeScheduler}
                  style={{ ...btnStyle, background: 'var(--crit)', color: '#fff' }}
                >
                  Remove
                </button>
              </div>
              <div style={{ 'font-size': '10px', color: 'var(--text-dim)', 'margin-top': '6px' }}>
                Valid range: {MIN_INTERVAL_SECONDS}s–{MAX_INTERVAL_SECONDS}s. Label:{' '}
                <code>dev.balakumar.ward.backup</code>{' · '}
                <Show when={scheduledInterval()} fallback={null}>
                  {(n) => <span>Currently runs every {n()}s.</span>}
                </Show>
              </div>
            </section>

            {/* Bus / error / info */}
            <Show when={busy()}>
              <div data-testid="backups-busy" style={{ color: 'var(--text-dim)' }}>{busy()}…</div>
            </Show>
            <Show when={error()}>
              <div data-testid="backups-error" style={{ color: 'var(--crit)' }}>{error()}</div>
            </Show>
            <Show when={info()}>
              <div data-testid="backups-info" style={{ color: 'var(--ok)' }}>{info()}</div>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}

const cellLabel = {
  padding: '4px 6px', color: 'var(--text-dim)', width: '140px', 'vertical-align': 'top',
} as const;
const cellValue = {
  padding: '4px 6px',
} as const;
const btnStyle = {
  padding: '4px 12px',
  'font-size': '11px',
  border: '1px solid var(--border)',
  background: 'var(--surface)',
  color: 'var(--text)',
  'border-radius': 'var(--radius)',
  cursor: 'pointer',
} as const;
const btnPrimary = {
  ...btnStyle,
  background: 'var(--accent)',
  color: '#000',
} as const;
const inputStyle = {
  flex: '1 1 240px',
  padding: '4px 8px',
  'font-size': '11px',
  border: '1px solid var(--border)',
  background: 'var(--surface)',
  color: 'var(--text)',
  'border-radius': 'var(--radius)',
  'min-width': '240px',
} as const;
