/**
 * Dashboard smoke test (Plan 12).
 *
 * Ports the behavioral contract of CCO's tests/e2e/dashboard.spec.mjs to
 * a WebDriver + tauri-driver session:
 *   - app launches without console errors
 *   - sidebar renders the 5 modes Ward supports
 *   - Organizer mode renders 12 category labels from the harness scan
 *
 * Runs against a real Ward.app via tauri-driver. Not wired into CI; the
 * user launches the app locally and runs `npm run test:e2e:smoke`.
 */

import { expect, browser } from '@wdio/globals';

describe('Dashboard smoke', () => {
  it('renders the 5 sidebar modes without console errors', async () => {
    // Collect any console errors so a regression in the renderer fails
    // the smoke test the same way it would in CCO's dashboard spec.
    const errors: string[] = [];
    await browser.on('console', (msg: { type: () => string; text: () => string }) => {
      if (msg.type() === 'error') errors.push(msg.text());
    });

    // Tauri's webview surfaces the Shell component which renders the
    // sidebar with `data-testid="sidebar"`.
    await expect($('[data-testid="sidebar"]')).toBeDisplayed();

    // Ward ships exactly five modes (Organizer, Security, Context Budget,
    // Sessions, Backups). Mode labels come from src/components/Sidebar.tsx.
    const expectedModes = [
      'Organizer',
      'Security',
      'Context Budget',
      'Sessions',
      'Backups',
    ];
    for (const label of expectedModes) {
      const node = await $(`[data-testid="sidebar"] >> text=${label}`);
      await expect(node).toBeDisplayed();
    }

    // The organizer is the default mode — verify the scan-loading sentinel
    // either disappears or was never shown.
    await expect($('[data-testid="scan-loading"]')).not.toBeDisplayed();

    // No console errors means the renderer is healthy.
    expect(errors).toEqual([]);
  });

  it('renders the 12 Organizer category labels', async () => {
    // Wait for the first scan resource to settle — sidebar is rendered
    // synchronously but categories come from the backend scan and need
    // the resource to resolve.
    await browser.waitUntil(
      async () => (await $$('[data-testid="sidebar"] >> text=Organizer').length) > 0,
      { timeout: 15_000, timeoutMsg: 'Sidebar never rendered' },
    );

    // Switch to Organizer explicitly (the default but be defensive in case
    // a future test left the harness in another mode).
    await $('[data-testid="sidebar"] >> text=Organizer').click();

    // The 12 category labels mirror `category_label()` in
    // src-tauri/src/harness/framework.rs:
    //   Skills, Memories, MCP, Commands, Agents, Plans, Rules, Config,
    //   Hooks, Plugins, Sessions, Settings
    const expectedCategories = [
      'Skills', 'Memories', 'MCP', 'Commands', 'Agents',
      'Plans', 'Rules', 'Config', 'Hooks', 'Plugins',
      'Sessions', 'Settings',
    ];

    for (const label of expectedCategories) {
      const cat = await $(`text=${label}`);
      await expect(cat).toBeDisplayed();
    }
  });
});
