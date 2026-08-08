import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/battleship/comms.html';

test('comms console: renders contacts and the most recent unread thread', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    window.__updateConsole('comms', JSON.stringify({
      comms: {
        messages: [
          { id: 'demo-msg-1', sender_name: 'Outpost Theta', body: 'We are under attack', responses: ['Acknowledged'], is_read: false },
          { id: 'demo-msg-0', sender_name: 'Relay Seven', body: 'Signal relay stable', responses: [], is_read: true },
        ],
        contacts: [
          { id: 'theta', name: 'Outpost Theta', in_range: true, stance: 'friendly' },
          { id: 'relay', name: 'Relay Seven', in_range: true, stance: 'neutral' },
        ],
      },
    }));
  });

  await expect(page.locator('ph-comms-contact-list .pill')).toHaveCount(2);
  await expect(page.locator('ph-comms-current-message #sender-label')).toHaveText('Outpost Theta');
  await expect(page.locator('ph-comms-current-message #messages')).toContainText('We are under attack');
  await expect(page.locator('#footer-target')).toHaveText('Outpost Theta');
});

test('comms console: response buttons send respond_to_message for the active thread', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
    window.__updateConsole('comms', JSON.stringify({
      comms: {
        messages: [
          {
            id: 'dr-myst-briefing',
            sender_name: 'Dr. Myst',
            body: 'Ardent, this is Dr. Myst at the Research Outpost. Whatever is out there is charging.',
            responses: ['What happens if it fires?', 'Is there anything unusual about the signal?'],
            selected_response: null,
            is_read: false,
          },
        ],
        contacts: [{ id: 'research-uuid', name: 'Research Outpost', in_range: true, stance: 'friendly' }],
      },
    }));
  });

  await expect(page.locator('ph-comms-current-message .resp-btn')).toHaveCount(2);
  await page.locator('ph-comms-current-message .resp-btn').first().click();

  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toEqual({
    action: 'respond_to_message',
    console: 'comms',
    message_id: 'dr-myst-briefing',
    response_index: 0,
  });
});
