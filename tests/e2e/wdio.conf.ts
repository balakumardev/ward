// ── WebDriverIO config for Ward ──────────────────────────────────────────
//
// Drives the production-grade Tauri app via `tauri-driver`. We deliberately
// point WebDriver at a long-running Tauri process the user launches with:
//     npm run tauri dev -- --no-watch
// or the packaged app launched with:
//     src-tauri/dist/sign.sh && open src-tauri/target/release/bundle/macos/Ward.app
//
// Specs run on the host's WebDriver port (4444 by default for tauri-driver).
// They are NOT expected to pass in CI — the user runs them on a real machine
// against a real `.app`. The tests just need to compile cleanly.
//
// See README.md in this folder for the full run workflow.

export const config: WebdriverIO.Config = {
  // Spawn tauri-driver (a small WebDriver server that exposes the Tauri
  // webview). Tauri's webview is WebKit-based, so we don't need a separate
  // browser binary.
  hostname: '127.0.0.1',
  port: 4444,
  path: '/',

  framework: 'mocha',
  reporters: ['spec'],

  // Each spec should be self-contained. 5 minutes is enough for the cold
  // start of a Tauri app + a few interactions.
  mochaOpts: {
    ui: 'bdd',
    timeout: 5 * 60 * 1000,
  },

  // One WebDriver session per spec file keeps state isolated.
  maxInstances: 1,

  // Tauri exposes the loaded window via webview context.
  capabilities: [
    {
      // The actual Tauri webview is WebKit; webdriverio tags it as `tauri`.
      // `tauri:options` is read by tauri-driver.
      browserName: 'tauri',
      'tauri:options': {
        application: process.env.WARD_APP_PATH ||
          'src-tauri/target/release/bundle/macos/Ward.app',
      },
    },
  ],

  specs: [
    './smoke.spec.ts',
    './scan.spec.ts',
    './move.spec.ts',
    './budget.spec.ts',
    './sessions.spec.ts',
    './backup.spec.ts',
    './security.spec.ts',
  ],
};

export default config;
