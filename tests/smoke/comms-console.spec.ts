// Comms console portrait navigation regression.
//
// The static iframe console should behave like a phone inbox:
// select a message to replace the list with the message, then Back should
// deselect the message and return to the message list.

import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/comms-console.html';

test('comms console: portrait Back deselects the open message', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(CONSOLE_URL);

  await expect(page.locator('#inbox-list .msg-row')).toHaveCount(2);
  await page.locator('#inbox-list .msg-row').first().click();

  await expect(page.locator('.panel-mid')).toHaveClass(/portrait-message/);
  await expect(page.locator('#selected-msg')).toHaveAttribute('data-id', 'demo-msg-1');
  await expect(page.locator('#chat-body')).toContainText('We are under attack');
  await expect(page.locator('#inbox-list .msg-row.selected')).toHaveCount(1);

  await page.locator('.back-btn').click();

  await expect(page.locator('.panel-mid')).not.toHaveClass(/portrait-message/);
  await expect(page.locator('#selected-msg')).toHaveAttribute('data-id', '');
  await expect(page.locator('#chat-placeholder')).toHaveText('SELECT A MESSAGE');
  await expect(page.locator('#inbox-list .msg-row.selected')).toHaveCount(0);
});
