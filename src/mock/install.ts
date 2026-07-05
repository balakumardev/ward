// Dev-only installer for the mock Tauri IPC bridge.
//
// Tauri v2's `@tauri-apps/api/core::invoke` is literally:
//     window.__TAURI_INTERNALS__.invoke(cmd, args, options)
// and `api.ts::isTauri()` checks for that same global. So defining
// `window.__TAURI_INTERNALS__` here makes the ENTIRE UI believe it's running
// inside the native webview, with every command served by the in-memory mock
// store — no changes to api.ts or any production logic.
//
// This module is only imported when `import.meta.env.VITE_WARD_MOCK` is set
// (via `npm run dev:mock`, port 1430). Normal `tauri dev` and production
// builds never load it.

import { mockInvoke } from './dispatch';

const w = window as unknown as { __TAURI_INTERNALS__?: unknown };

w.__TAURI_INTERNALS__ = {
  invoke: (cmd: string, args?: Record<string, unknown>) => mockInvoke(cmd, args ?? {}),
  // The following members exist on the real object; api.ts doesn't use them,
  // but we stub them so any incidental access is safe.
  transformCallback: (cb: unknown) => cb,
  convertFileSrc: (filePath: string) => filePath,
  metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
};

// eslint-disable-next-line no-console
console.info(
  '%c[ward-mock]%c Tauri bridge mocked — UI driven by fixture + real scan data',
  'color:#8ff0a8;font-weight:bold',
  'color:#8ff0a8',
);
