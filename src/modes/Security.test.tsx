import { vi, test, expect, beforeEach } from 'vitest';

// Mock the Tauri event API and the api module before importing the
// component so the imports resolve to our fakes.
const listenMock = vi.fn();
vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

vi.mock('../api', () => ({
  api: {
    securityScan: vi.fn(),
  },
}));

import { render } from '@solidjs/testing-library';
import { Security } from './Security';
import { api } from '../api';

beforeEach(() => {
  listenMock.mockReset();
  listenMock.mockResolvedValue(() => {});
  (api.securityScan as unknown as ReturnType<typeof vi.fn>).mockReset();
  (api.securityScan as unknown as ReturnType<typeof vi.fn>).mockResolvedValue({
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
  });
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
