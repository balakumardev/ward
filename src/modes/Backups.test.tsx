import { test, expect } from 'vitest';
import { render } from '@solidjs/testing-library';
import { Backups } from './Backups';
import type { BackupStatus, GitLogEntry } from '../api';

const STATUS: BackupStatus = {
  hasRepo: true,
  lastCommit: '9a8b7c6d5e',
  lastCommitAt: '2026-07-05T09:20:00Z',
  schedulerInstalled: false,
  schedulerOrphaned: false,
  schedulerInterval: null,
  remoteUrl: null,
};

const LOG: GitLogEntry[] = [
  { sha: '9a8b7c6d5e4f3a2b', subject: 'backup: ward (claude) 2026-07-05', author: 'ward', committedAt: '2026-07-05T09:20:00Z' },
  { sha: '1c2d3e4f5a6b7c8d', subject: 'backup: ward sync 2026-07-05', author: 'ward', committedAt: '2026-07-05T04:00:00Z' },
];

// The Backups component takes its whole api surface via props, so tests
// can inject a fake without mocking the `../api` module.
function makeApi(overrides: Record<string, unknown> = {}) {
  return {
    backupStatus: () => Promise.resolve(STATUS),
    backupRun: () => Promise.resolve({ filesCopied: 0, bytesCopied: 0, skipped: [] }),
    backupSync: () => Promise.resolve({ committed: false, sha: null, message: '', committedAt: null }),
    backupPush: () => Promise.resolve({ pushed: false, reason: '', remoteUrl: null }),
    backupSchedulerInstall: () => Promise.resolve(),
    backupSchedulerRemove: () => Promise.resolve(),
    backupSetRemote: () => Promise.resolve(),
    backupLog: () => Promise.resolve(LOG),
    ...overrides,
  };
}

test('Backups renders history rows from backupLog', async () => {
  const { findAllByTestId, getByTestId } = render(() => (
    <Backups scan={{} as never} api={makeApi() as never} />
  ));
  const rows = await findAllByTestId('backups-history-row');
  expect(rows.length).toBe(2);
  expect(getByTestId('backups-history')).toBeTruthy();
  // Short sha + subject rendered in the first row.
  expect(rows[0].textContent).toContain('9a8b7c6d5e');
  expect(rows[0].textContent).toContain('backup: ward (claude)');
});

test('Backups shows empty-history hint when backupLog is empty', async () => {
  const { findByTestId } = render(() => (
    <Backups scan={{} as never} api={makeApi({ backupLog: () => Promise.resolve([]) }) as never} />
  ));
  expect(await findByTestId('backups-history-empty')).toBeTruthy();
});

test('Backups enables Remove for an orphaned scheduler', async () => {
  const orphanStatus: BackupStatus = { ...STATUS, schedulerInstalled: false, schedulerOrphaned: true };
  const { findByTestId } = render(() => (
    <Backups scan={{} as never} api={makeApi({ backupStatus: () => Promise.resolve(orphanStatus) }) as never} />
  ));
  // Orphan badge is shown …
  expect(await findByTestId('backups-scheduler-orphaned')).toBeTruthy();
  // … and Remove is NOT disabled, so the orphan can be cleared.
  const remove = (await findByTestId('backups-scheduler-remove')) as HTMLButtonElement;
  expect(remove.disabled).toBe(false);
});
