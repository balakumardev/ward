import { createMemo, createResource, createSignal, Show } from 'solid-js';
import type {
  BackupStatus,
  CommitInfo,
  ExportReport,
  PushResult,
  ScanResult,
} from '../api';
import '../styles/backups.css';

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
    <div class="bk" data-testid="backups-mode">
      <div class="bk-header">
        <h2 class="bk-title">Backups</h2>
      </div>

      <Show when={status()} fallback={
        <div class="bk-loading" data-testid="backups-loading">Loading backup status…</div>
      }>
        {(s) => (
          <div class="bk-grid rise" data-testid="backups-panel">
            {/* Status card */}
            <section class="bk-card bk-status" data-testid="backups-status-panel">
              <div class="bk-card-head">
                <span class="bk-card-title">Status</span>
              </div>
              <div class="bk-fields">
                <div class="bk-field">
                  <span class="bk-field-label">Repo</span>
                  <span class="bk-field-value">
                    {s().hasRepo
                      ? <span class="badge badge-ok" data-testid="backups-repo-present">present (~/.ward-backups/)</span>
                      : <span class="badge badge-warn" data-testid="backups-repo-missing">not yet initialized</span>}
                  </span>
                </div>
                <div class="bk-field">
                  <span class="bk-field-label">Last commit</span>
                  <span class="bk-field-value">
                    <span class="bk-sha" data-testid="backups-last-commit">{shortSha(s().lastCommit)}</span>
                    {' · '}
                    <span class="bk-date">{fmtDate(s().lastCommitAt)}</span>
                  </span>
                </div>
                <div class="bk-field">
                  <span class="bk-field-label">Remote</span>
                  <span class="bk-field-value">
                    <Show when={s().remoteUrl} fallback={<span class="bk-muted">none</span>}>
                      {(u) => <code class="bk-mono" data-testid="backups-remote">{u()}</code>}
                    </Show>
                  </span>
                </div>
                <div class="bk-field">
                  <span class="bk-field-label">Scheduler</span>
                  <span class="bk-field-value">
                    <Show when={s().schedulerInstalled} fallback={
                      <Show when={s().schedulerOrphaned} fallback={
                        <span class="bk-not-installed" data-testid="backups-scheduler-not-installed">
                          not installed
                        </span>
                      }>
                        <span class="badge badge-warn" data-testid="backups-scheduler-orphaned">
                          orphaned — plist missing, click Remove to clear
                        </span>
                      </Show>
                    }>
                      <span class="badge badge-ok" data-testid="backups-scheduler-installed">
                        installed · {s().schedulerInterval}s
                      </span>
                    </Show>
                  </span>
                </div>
              </div>
            </section>

            {/* Run / Sync / Push */}
            <section class="bk-card">
              <div class="bk-card-head">
                <span class="bk-card-title">Manual backup</span>
              </div>
              <div class="bk-actions">
                <button class="btn btn-primary" data-testid="backups-btn-run" disabled={busy() !== null} onClick={runBackup}>
                  Run backup
                </button>
                <button class="btn btn-primary" data-testid="backups-btn-sync" disabled={busy() !== null} onClick={syncBackup}>
                  Sync (commit)
                </button>
                <button class="btn btn-ghost" data-testid="backups-btn-push" disabled={busy() !== null} onClick={pushBackup}>
                  Push
                </button>
              </div>
              <div class="bk-hint">
                Push is the only network action — requires an explicit click.
              </div>
            </section>

            {/* Remote URL */}
            <section class="bk-card">
              <div class="bk-card-head">
                <span class="bk-card-title">Remote</span>
              </div>
              <div class="bk-form-row">
                <input
                  class="bk-input"
                  data-testid="backups-remote-input"
                  type="text"
                  placeholder="git@github.com:you/ward-backups.git"
                  value={remoteDraft()}
                  onInput={(e) => setRemoteDraft(e.currentTarget.value)}
                />
                <button class="btn btn-ghost" data-testid="backups-remote-set" disabled={busy() !== null} onClick={setRemote}>
                  Set remote
                </button>
              </div>
              <Show when={s().remoteUrl}>
                <div class="bk-hint">
                  Current: <code class="bk-mono">{s().remoteUrl}</code>
                </div>
              </Show>
            </section>

            {/* Scheduler */}
            <section class="bk-card" data-testid="backups-scheduler-panel">
              <div class="bk-card-head">
                <span class="bk-card-title">Scheduler (launchd)</span>
              </div>
              <div class="bk-form-row">
                <label class="bk-label">
                  Interval (sec):
                  <input
                    class="bk-input bk-input-num"
                    data-testid="backups-interval-input"
                    type="number"
                    min={MIN_INTERVAL_SECONDS}
                    max={MAX_INTERVAL_SECONDS}
                    value={intervalDraft()}
                    onInput={(e) => setIntervalDraft(Number(e.currentTarget.value) || MIN_INTERVAL_SECONDS)}
                  />
                </label>
                <button
                  class="btn btn-ghost"
                  data-testid="backups-scheduler-install"
                  disabled={busy() !== null}
                  onClick={installScheduler}
                >
                  Install
                </button>
                <button
                  class="btn btn-danger"
                  data-testid="backups-scheduler-remove"
                  disabled={busy() !== null || (!s().schedulerInstalled && !s().schedulerOrphaned)}
                  onClick={removeScheduler}
                >
                  Remove
                </button>
              </div>
              <div class="bk-hint">
                Valid range: {MIN_INTERVAL_SECONDS}s–{MAX_INTERVAL_SECONDS}s. Label:{' '}
                <code class="bk-mono">dev.balakumar.ward.backup</code>{' · '}
                <Show when={scheduledInterval()} fallback={null}>
                  {(n) => <span>Currently runs every {n()}s.</span>}
                </Show>
              </div>
            </section>

            {/* Bus / error / info */}
            <Show when={busy()}>
              <div class="bk-msg bk-msg-busy" data-testid="backups-busy">{busy()}…</div>
            </Show>
            <Show when={error()}>
              <div class="bk-msg bk-msg-err" data-testid="backups-error">{error()}</div>
            </Show>
            <Show when={info()}>
              <div class="bk-msg bk-msg-ok" data-testid="backups-info">{info()}</div>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}
