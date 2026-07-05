/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Dev-only flag. Set by `npm run dev:mock` to install the mock Tauri IPC
   *  bridge (src/mock/) so the full UI runs in a plain browser on fixture +
   *  real scan data. Never set under `tauri dev` or in production builds. */
  readonly VITE_WARD_MOCK?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
