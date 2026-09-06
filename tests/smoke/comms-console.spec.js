import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/battleship/comms.html';

// Issue #1380: on a phone the inbox IS the console — one row per thread behind
// the HAILS tab, contacts behind the other, and a thread opens as a LOCAL
// overlay you leave with Back. Nothing here is a Station Bar tab: the panel
// declares no `data-tab-code`, so the bar never advertises it.
test('comms console: threads on a phone, the conversation as a local overlay', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(CONSOLE_URL);

  await page.evaluate(() => {
    // The battleship Comms console is a flat `comms`-family payload: the fields
    // sit at the top level, exactly as buildCommsConsoleState emits them (and as
    // tests/client/comms-console.test.js drives renderStation). Nesting them
    // under a `comms` key is a shape the runtime never sends.
    //
    // Theta's five lines are deliberately long: the history has to OVERFLOW its
    // box on a phone for the open-on-newest-line assertion below to mean
    // anything.
    window.__updateConsole('comms', JSON.stringify({
      messages: [
        { id: 'theta-0', thread_id: 'theta', sender_uuid: 'theta', sender_name: 'Outpost Theta', body: 'Phoenix, Outpost Theta. Are you receiving? Our long-range array has been down since the last transit and we are running on the backup transponder alone, which gives us a tenth of the gain and none of the directionality we would want for a challenge at this range. If you are hearing this at all it is because you are already inside the shell, and we would rather waste your time than find out later that nobody was listening. We have been calling on the hour since the array went, and the log says nobody has answered once, so forgive the length of this: it is easier to say everything now than to guess what you need.', responses: [], is_read: true },
        { id: 'theta-1', thread_id: 'theta', sender_uuid: 'theta', sender_name: 'Outpost Theta', body: 'Reading three hulls on approach from the outer marker, none of them squawking, none of them on any filed course we hold. Closing steadily, in formation, at a rate that says they know exactly where the station is. We do not have the power budget to hold a continuous scan on them, and the reactor will not carry both the shutters and the array, so we are choosing the shutters. Best guess on mass puts the lead one somewhere between a cutter and a light escort, and the other two smaller and faster, which is not a shape any survey outfit flies.', responses: [], is_read: true },
        { id: 'theta-2', thread_id: 'theta', sender_uuid: 'theta', sender_name: 'Outpost Theta', body: 'They have crossed the inner marker without answering a single challenge on any band we can still transmit on. Our shutters are down, the docking clamps are locked out from the control room, and we have moved everyone who is not sitting at a console into the lower ring. The ring is rated for a hull breach on one face only, which is the part nobody wants to say out loud. If they board, they board through the service lock on the far side, and we cannot see it from here now that the array is off.', responses: [], is_read: true },
        { id: 'theta-3', thread_id: 'theta', sender_uuid: 'theta', sender_name: 'Outpost Theta', body: 'Lead hull is running out its mounts and the two behind it are spreading to bracket us. Whatever this is, it is not a survey flight and it is not a navigation mistake. Requesting immediate assistance from any hull in range, on any heading, at any speed you can make. We are a repair depot with a welding laser and eleven people, and none of that is going to matter in ten minutes. If you cannot reach us in time, relay this transcript onward so somebody knows which way they came from and which way they left.', responses: [], is_read: true },
        { id: 'theta-4', thread_id: 'theta', sender_uuid: 'theta', sender_name: 'Outpost Theta', body: 'We are under attack', responses: ['Acknowledged'], is_read: false },
        { id: 'relay-0', thread_id: 'relay', sender_uuid: 'relay', sender_name: 'Relay Seven', body: 'Signal relay stable', responses: [], is_read: true },
      ],
      contacts: [
        { uuid: 'theta', name: 'Outpost Theta', in_range: true, stance: 'friendly' },
        { uuid: 'relay', name: 'Relay Seven', in_range: true, stance: 'neutral' },
      ],
    }));
  });

  // Two threads, six messages: the five-message conversation is ONE row
  // previewing its latest line, and the HAILS tab carries the unread count.
  const rows = page.locator('ph-comms-hail-list .row');
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText('Outpost Theta');
  await expect(rows.first()).toContainText('We are under attack');
  await expect(rows.first().locator('.count')).toHaveText('5');
  await expect(page.locator('#comms-hails-unread')).toHaveText('1');
  await expect(page.locator('#footer-target')).toHaveText('Outpost Theta');

  // The thread is not on screen until a row is tapped: the list owns the phone.
  const panel = page.locator('#comms-thread-panel');
  await expect(panel).toBeHidden();
  expect(await panel.getAttribute('data-tab-code')).toBeNull();

  await rows.first().click();
  await expect(panel).toBeVisible();
  const thread = page.locator('ph-comms-current-message #messages');
  await expect(thread).toContainText('Are you receiving?');
  await expect(thread).toContainText('We are under attack');
  await expect(page.locator('ph-comms-current-message #sender-label')).toHaveText('Outpost Theta');
  // The responses are pinned OUTSIDE the scrolling history, and the console
  // itself has not grown a scrollbar.
  await expect(page.locator('ph-comms-current-message .responses .resp-btn')).toHaveCount(1);
  expect(await page.evaluate(
    () => document.documentElement.scrollHeight <= document.documentElement.clientHeight,
  )).toBe(true);

  // A conversation opens on its NEWEST line — the one the pinned responses
  // answer — not at the top of a history the operator has already read. The
  // overlay is display:none on the render that fills it, and the responses row
  // is created in that same render, so both of those have to be accounted for
  // before the box is measured.
  const scroll = await thread.evaluate((box) => ({
    top: box.scrollTop, height: box.scrollHeight, client: box.clientHeight,
  }));
  expect(scroll.height).toBeGreaterThan(scroll.client);
  expect(scroll.top).toBeGreaterThanOrEqual(scroll.height - scroll.client - 1);

  await page.locator('#comms-thread-back').click();
  await expect(panel).toBeHidden();

  // The other tab is the contact list, in the same area.
  await page.locator('#comms-seg-contacts').click();
  await expect(page.locator('ph-comms-contact-list .pill')).toHaveCount(2);
  await expect(rows.first()).toBeHidden();
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

// Landscape/desktop: the thread sits beside the list, so no overlay is
// involved and the three input routes converge on one rendered selection.
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
  const panel = page.locator('#comms-thread-panel');
  const openOverlays = page.locator('.overlay-panel.open');
  // Mirror of the portrait assertion above: there the thread panel IS an
  // overlay and gets marked open, here it is an ordinary second column that is
  // on screen the whole time. `.overlay-panel.open` is the fact console-core
  // posts to the shell as this console's open overlay, and the Station Bar
  // spends the seat's own tab returning from an overlay instead of opening the
  // per-system damage popup (issue #1374) — so a panel that covers nothing must
  // never claim one, before or after a row is activated.
  await expect(panel).toBeVisible();
  await expect(openOverlays).toHaveCount(0);

  await rows.nth(1).click();
  await expect(rows.nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(sender).toHaveText('Bravo');
  await expect(panel).toBeVisible();
  await expect(openOverlays).toHaveCount(0);

  await page.keyboard.press('m');
  await expect(rows.nth(0)).toHaveAttribute('aria-selected', 'true');
  await expect(sender).toHaveText('Alpha');
  await expect(openOverlays).toHaveCount(0);

  await page.evaluate(() => window.activateSemanticAction('comms.select-message', {
    source: 'gamepad',
  }));
  await expect(rows.nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(sender).toHaveText('Bravo');
  await expect(openOverlays).toHaveCount(0);
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
