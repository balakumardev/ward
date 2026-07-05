import { vi, test, expect, beforeEach } from 'vitest';

// Mock the Tauri event API and the api module before importing the
// component so the imports resolve to our fakes.
const listenMock = vi.fn();
vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

// Plan 15 — after each scan, Security pushes the critical count to the
// native dock badge + tray tooltip via `api.nativeUpdateStatus`, guarded
// by `isTauri()`. Mock both so the effect runs under jsdom (and so the
// existing tests, which now also trigger that effect on mount, keep
// passing).
const nativeUpdateStatusMock = vi.fn();
const isTauriMock = vi.fn(() => true);
vi.mock('../api', () => ({
  api: {
    securityScan: vi.fn(),
    nativeUpdateStatus: (...args: unknown[]) => nativeUpdateStatusMock(...args),
  },
  isTauri: () => isTauriMock(),
}));

import { render, waitFor } from '@solidjs/testing-library';
import { Security } from './Security';
import { api } from '../api';

const SCAN_RESULT = {
  timestamp: '2026-01-01T00:00:00Z',
  servers: [],
  findings: [],
  duplicates: [],
  baselineDiffs: [],
  severityCounts: { critical: 0, high: 0, medium: 0, low: 0 },
  totalTools: 0,
  totalServers: 0,
  serversConnected: 0,
  serversFailed: 0,
  judgeUsed: false,
};

beforeEach(() => {
  listenMock.mockReset();
  listenMock.mockResolvedValue(() => {});
  nativeUpdateStatusMock.mockReset();
  isTauriMock.mockReset();
  isTauriMock.mockReturnValue(true);
  (api.securityScan as unknown as ReturnType<typeof vi.fn>).mockReset();
  (api.securityScan as unknown as ReturnType<typeof vi.fn>).mockResolvedValue(SCAN_RESULT);
});

test('Security subscribes to config-changed and scan-now events', async () => {
  const items: never[] = [];
  render(() => <Security items={items} api={{} as never} />);
  // The listener registrations are async; wait a microtask.
  await Promise.resolve();
  await Promise.resolve();

  const events = listenMock.mock.calls.map((c) => c[0]);
  expect(events).toContain('config-changed');
  expect(events).toContain('scan-now');
});

test('Security calls securityScan on mount', async () => {
  const items: never[] = [];
  render(() => <Security items={items} api={{} as never} />);
  await Promise.resolve();
  expect(api.securityScan).toHaveBeenCalled();
});

test('Security pushes critical count to the native badge after a scan', async () => {
  (api.securityScan as unknown as ReturnType<typeof vi.fn>).mockResolvedValue({
    ...SCAN_RESULT,
    severityCounts: { critical: 1, high: 0, medium: 0, low: 0 },
  });
  const items: never[] = [];
  render(() => <Security items={items} api={{} as never} />);
  await waitFor(() =>
    expect(nativeUpdateStatusMock).toHaveBeenCalledWith(1, expect.any(String)),
  );
});

test('Security does not push native status outside Tauri', async () => {
  isTauriMock.mockReturnValue(false);
  const items: never[] = [];
  render(() => <Security items={items} api={{} as never} />);
  // Let the scan resolve + the effect run.
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  expect(nativeUpdateStatusMock).not.toHaveBeenCalled();
});
