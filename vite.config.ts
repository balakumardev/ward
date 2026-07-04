import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [solid()],
  // Tauri expects a fixed port, fail if unavailable
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test-setup.ts'],
    // Plan 12 — exclude the WebDriver E2E specs from vitest. They have
    // their own runner (webdriverio + tauri-driver) and rely on
    // @wdio/globals which isn't installed at the root.
    exclude: ['**/node_modules/**', 'tests/e2e/**'],
  },
});