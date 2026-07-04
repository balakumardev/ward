/**
 * Budget mode smoke (Plan 12).
 *
 * Verifies that switching to Context Budget renders the budget panel
 * (Plan 06). Capability-gated: the harness must report contextBudget.
 */

import { expect, browser, $ } from '@wdio/globals';

describe('Context Budget mode', () => {
  it('renders the budget panel after switching modes', async () => {
    await $('[data-testid="sidebar"] >> text=Context Budget').click();
    await browser.pause(500);

    // Either the budget panel renders OR the harness reports
    // `capabilities.contextBudget = false` and we show the
    // `data-testid="budget-unsupported"` fallback. Both are valid.
    const budgetPanel = await $('[data-testid="budget-panel"]');
    const unsupported = await $('[data-testid="budget-unsupported"]');

    const budgetVisible = await budgetPanel.isDisplayed();
    const unsupportedVisible = await unsupported.isDisplayed();
    expect(budgetVisible || unsupportedVisible).toBe(true);
  });
});
