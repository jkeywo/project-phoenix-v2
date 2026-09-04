import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/battleship/comms.html';

test('comms console: renders contacts and the most recent unread thread', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    // The battleship Comms console is a flat `comms`-family payload: the fields
    // sit at the top level, exactly as buildCommsConsoleState emits them (and as
    // tests/client/comms-console.test.js drives renderStation). Nesting them
    // under a `comms` key is a shape the runtime never sends.
    window.__updateConsole('comms', JSON.stringify({
      messages: [
        { id: 'demo-msg-1', sender_name: 'Outpost Theta', body: 'We are under attack', responses: ['Acknowledged'], is_read: false },
        { id: 'demo-msg-0', sender_name: 'Relay Seven', body: 'Signal relay stable', responses: [], is_read: true },
      ],
      contacts: [
        { uuid: 'theta', name: 'Outpost Theta', in_range: true, stance: 'friendly' },
        { uuid: 'relay', name: 'Relay Seven', in_range: true, stance: 'neutral' },
      ],
    }));
  });

  await expect(page.locator('ph-comms-contact-list .pill')).toHaveCount(2);
  await expect(page.locator('ph-comms-current-message #sender-label')).toHaveText('Outpost Theta');
  await expect(page.locator('ph-comms-current-message #messages')).toContainText('We are under attack');
  await expect(page.locator('#footer-target')).toHaveText('Outpost Theta');
});

test('comms console: response buttons send respond_to_message for the active thread', { tag: '@core' }, async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
    window.__updateConsole('comms', JSON.stringify({
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
      contacts: [{ uuid: 'research-uuid', name: 'Research Outpost', in_range: true, stance: 'friendly' }],
    }));
  });

  await expect(page.locator('ph-comms-current-message .resp-btn')).toHaveCount(2);
  await page.locator('ph-comms-current-message .resp-btn').first().click();

  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toMatchObject({
    action: 'respond_to_message',
    console: 'comms',
    message_id: 'dr-myst-briefing',
    response_index: 0,
    semantic_action: 'comms.respond',
    correlation: expect.any(String),
  });
});

test('comms console: pointer, keyboard and gamepad share one local message selection', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(JSON.parse(json));
    window.__updateConsole('comms', JSON.stringify({
      messages: [
        { id: 'm1', sender_name: 'Alpha', body: 'First message', is_read: false },
        { id: 'm2', sender_name: 'Bravo', body: 'Second message', is_read: true },
      ],
      contacts: [],
    }));
  });

  const rows = page.locator('ph-comms-hail-list .row');
  const sender = page.locator('ph-comms-current-message #sender-label');
  await rows.nth(1).click();
  await expect(rows.nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(sender).toHaveText('Bravo');

  await page.keyboard.press('m');
  await expect(rows.nth(0)).toHaveAttribute('aria-selected', 'true');
  await expect(sender).toHaveText('Alpha');

  await page.evaluate(() => window.activateSemanticAction('comms.select-message', {
    source: 'gamepad',
  }));
  await expect(rows.nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(sender).toHaveText('Bravo');
  expect(await page.evaluate(() => window.__sent)).toEqual([]);
});

for (const hull of ['battleship', 'cruiser']) {
  test(`${hull} comms: unavailable response stays disabled and a forced submission shows Refused`, async ({ page }) => {
    await page.goto(`/gui/${hull}/comms.html`);
    await page.evaluate((variant) => {
      window.__sent = [];
      window.__sendAction = (json) => window.__sent.push(JSON.parse(json));
      const comms = {
        messages: [{
          id: 'unavailable-response', sender_name: 'Relay Seven', body: 'Signal fading.',
          responses: [{ text: 'Acknowledge', available: false, important: false }],
          selected_response: null, is_read: false,
        }],
        contacts: [{ uuid: 'relay', name: 'Relay Seven', in_range: false, stance: 'neutral' }],
      };
      const state = variant === 'cruiser' ? {
        systems: { radio_port: comms },
        system_ids: ['radio_port'],
        system_families: { radio_port: 'comms' },
      } : comms;
      window.__updateConsole('comms', JSON.stringify(state));
    }, hull);

    const response = page.locator('ph-comms-current-message .resp-btn');
    await expect(response).toBeDisabled();
    await response.click({ force: true });
    expect(await page.evaluate(() => window.__sent)).toEqual([]);

    const forced = await page.evaluate(() => {
      window.activateSemanticAction('comms.respond', {
        source: 'control',
        detail: { message_id: 'unavailable-response', response_index: 0 },
      });
      return window.__sent.at(-1);
    });
    expect(forced).toMatchObject({
      action: 'respond_to_message',
      message_id: 'unavailable-response',
      response_index: 0,
      semantic_action: 'comms.respond',
      correlation: expect.any(String),
    });

    await page.evaluate((correlation) => {
      window.__updateActionFeedback({ correlation, state: 'Refused' });
    }, forced.correlation);
    const feedback = page.locator(
      '.semantic-action-feedback__item[data-action-id="comms.respond"]',
    );
    await expect(feedback).toHaveAttribute('data-state', 'Refused');
  });
}
