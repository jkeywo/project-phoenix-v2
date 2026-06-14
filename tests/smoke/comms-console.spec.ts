// Comms console portrait navigation regression.
//
// The static iframe console should behave like a phone inbox:
// select a message to replace the list with the message, then Back should
// deselect the message and return to the message list.

import { test, expect } from './fixtures';

const CONSOLE_URL = '/client/gui/comms-console.html';

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

test('comms console: multi-speaker thread replies to latest active message', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    (window as any).__sentCommsActions = [];
    (window as any)._sendCommsAction = (action: string, payload: unknown) => {
      (window as any).__sentCommsActions.push({ action, payload });
    };
    (window as any).__updateConsole('Comms', JSON.stringify({
      messages: [
        {
          id: 'research-standby',
          sender_uuid: 'research-uuid',
          sender_name: 'Research Outpost',
          subject: 'A.E.V. Ardent, this is the Research',
          body: 'A.E.V. Ardent, this is the Research Outpost. We read you. Stand by - patching you through to Dr. Myst now.',
          responses: [],
          selected_response: null,
          is_read: true,
          is_orphaned: false,
          sender_in_range: true,
          thread_id: 'research-scholar',
          is_urgent: false,
        },
        {
          id: 'dr-myst-briefing',
          sender_uuid: 'research-uuid',
          sender_name: 'Dr. Myst',
          subject: 'Ardent, this is Dr. Myst at the Resear',
          body: 'Ardent, this is Dr. Myst at the Research Outpost. Whatever is out there is charging.',
          responses: ['What happens if it fires?', 'Is there anything unusual about the signal?'],
          selected_response: null,
          is_read: false,
          is_orphaned: false,
          sender_in_range: true,
          thread_id: 'research-scholar',
          is_urgent: true,
        },
      ],
      contacts: [{ uuid: 'research-uuid', name: 'Research Outpost', in_range: true, is_urgent: true }],
      objectives: [],
    }));
  });

  await expect(page.locator('#inbox-list .msg-row')).toHaveCount(1);
  await page.locator('#inbox-list .msg-row').click();

  await expect(page.locator('#selected-msg')).toHaveAttribute('data-id', 'research-scholar');
  await expect(page.locator('#chat-body')).toContainText('RESEARCH OUTPOST');
  await expect(page.locator('#chat-body')).toContainText('DR. MYST');
  await expect(page.locator('#response-buttons .response-btn')).toHaveCount(2);

  await page.locator('#response-buttons .response-btn').first().click();
  await expect.poll(async () => page.evaluate(() => (window as any).__sentCommsActions)).toContainEqual({
    action: 'respond_to_message',
    payload: { message_id: 'dr-myst-briefing', response_index: 0 },
  });
});
