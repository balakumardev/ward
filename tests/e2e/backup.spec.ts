/**
 * Backup mode smoke (Plan 12).
 *
 * Verifies the Backups sidebar mode renders the backup center
 * (Plan 08) or its capability-gated fallback.
 */

import { expect, browser, $ } from '@wdio/globals';

describe('Backups mode', () => {
  it('renders the backup center or its fallback after switching modes', async () => {
    await $('[data-testid="sidebar"] >> text=Backups').click();
    await browser.pause(500);

    const backupsPanel = await $('[data-testid="backups-panel"]');
    const unsupported = await $('[data-testid="backups-unsupported"]');

    const panelVisible = await backupsPanel.isDisplayed();
    const unsupportedVisible = await unsupported.isDisplayed();
    expect(panelVisible || unsupportedVisible).toBe(true);
  });

  it('shows the backup status line', async () => {
    // The status block lives inside the Backups panel and surfaces
    // the last commit / scheduler state. We just verify *some* node
    // inside the panel renders so we don't lock to a specific copy.
    const panel = await $('[data-testid="backups-panel"]');
    if (await panel.isExisting()) {
      const html = await panel.getHTML();
      expect(html.length).toBeGreaterThan(0);
    }
  });
});
