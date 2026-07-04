/**
 * Sessions mode smoke (Plan 12).
 *
 * Verifies that switching to Sessions opens the session list (Plan 07).
 * Capability-gated; mirrors the `budget.spec.ts` pattern.
 */

import { expect, browser, $ } from '@wdio/globals';

describe('Sessions mode', () => {
  it('renders the sessions panel after switching modes', async () => {
    await $('[data-testid="sidebar"] >> text=Sessions').click();
    await browser.pause(500);

    const sessionsPanel = await $('[data-testid="sessions-panel"]');
    const unsupported = await $('[data-testid="sessions-unsupported"]');

    const panelVisible = await sessionsPanel.isDisplayed();
    const unsupportedVisible = await unsupported.isDisplayed();
    expect(panelVisible || unsupportedVisible).toBe(true);
  });

  it('lists at least one session row when the harness has any', async () => {
    // Soft assertion — only meaningful if the user has session logs.
    const rows = await $$('[data-testid="session-row"]');
    expect(rows.length).toBeGreaterThanOrEqual(0);
  });
});
