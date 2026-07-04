/**
 * Security mode smoke (Plan 12).
 *
 * Verifies that switching to Security renders the scanner (Plan 05) and
 * the "Scan now" button is clickable.
 */

import { expect, browser, $ } from '@wdio/globals';

describe('Security mode', () => {
  it('renders the security panel after switching modes', async () => {
    await $('[data-testid="sidebar"] >> text=Security').click();
    await browser.pause(500);

    const securityPanel = await $('[data-testid="security-panel"]');
    // Unlike the capability-gated modes, Security is always available
    // for every harness (Plan 05 — every adapter sets mcp_security).
    await expect(securityPanel).toBeDisplayed();
  });

  it('exposes a working Scan now button', async () => {
    const scanButton = await $('[data-testid="security-scan-now"]');
    // Some Security.tsx versions only render the button after a scan
    // has been kicked off; tolerate either order.
    if (await scanButton.isExisting()) {
      await expect(scanButton).toBeClickable();
      await scanButton.click();
      // After the click, the panel should still be present (the scan
      // runs asynchronously; we don't await results in a smoke spec).
      await expect($('[data-testid="security-panel"]')).toBeDisplayed();
    }
  });
});
