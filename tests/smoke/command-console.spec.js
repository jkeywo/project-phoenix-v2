import { test, expect } from './fixtures';
import { ts } from './strings';

// End to end at the client boundary: the auxiliary Command console, driven by
// the `command` blackboard(s) a server publishes. The stance selection
// round-trip and the server-side application are pinned in the Rust
// admission/command tests and the pure `ship::command_stance` resolver; this
// spec pins the console's own contract.
//
// Issue #1381 turned the single flat block into one card per directable
// station (`{ red_alert, stations: [...] }` — `gui/console-state.js`'s
// `buildCommandConsoleState`), each card listing its STANDARD stances only
// plus one synthesized Default row that resolves to whichever alert-neutral
// fallback matches the client's own `red_alert`. These fixtures build the
// payload in that already-resolved shape (the shape the real builder emits),
// the way `command-console.test.js` does for `renderStation` directly — this
// spec drives the same contract through the real page instead.
//
// Prior art preserved from before that slice: it lists the directed
// Station's stances only while that Station is AI-controlled (#1107), shows
// a persistent non-colour automation cue and marks the stance in force
// (#1107), an uncrewed Command shows the AI-selected stance and a human
// taking the seat can re-pick (#1109), and a scenario-contributed stance
// appears and later withdraws (#1110).

const CONSOLE_URL = '/gui/command-console.html';

function aiDirectedState(stationOverrides = {}) {
  return {
    red_alert: false,
    stations: [{
      command_system_id: 'command',
      directed_station: 'tactical',
      directed_station_name: 'Tactical',
      directed_station_ai: true,
      command_auto: false,
      selected_stance: 'tactical-normal',
      stances: [
        { id: 'tactical-weapons-free', label: 'entity.alliance_destroyer.station.tactical.stance.weapons_free', kind: 'standard', high_alert: true },
        { id: 'tactical-hold', label: 'entity.alliance_destroyer.station.tactical.stance.hold', kind: 'standard', high_alert: false },
      ],
      default_stance: { id: 'tactical-normal', label: 'entity.alliance_destroyer.station.tactical.stance.normal', kind: 'normal_alert_neutral', high_alert: false },
      default_selected: true,
      ...stationOverrides,
    }],
  };
}

test('command console: one card per directable station, listing standard stances plus one Default row', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('command', JSON.stringify(s)), aiDirectedState());

  // Criterion 1: one card for the singular command_target this ship authors.
  await expect(page.locator('.command-card')).toHaveCount(1);
  await expect(page.locator('.directed-name').first()).toHaveText('Tactical');
  // Criterion 2: the two standard stances as ordinary buttons, the two
  // alert-neutral fallbacks folded into exactly one Default row.
  await expect(page.locator('.command-card .stance-btn:not(.default-row)')).toHaveCount(2);
  await expect(page.locator('.command-card .stance-btn.default-row')).toHaveCount(1);
  // Persistent non-colour cue: the directed station is AI and therefore
  // directable, so the buttons are enabled and the card is not greyed.
  await expect(page.locator('.station-cue-text').first()).toHaveText(ts('console.command.ai_directed'));
  await expect(page.locator('.command-card').first()).not.toHaveClass(/crewed/);
  await expect(page.locator('.command-card .stance-btn').first()).toBeEnabled();
  // The stance in force is marked (a glyph, not a colour) — here the Default
  // row, resolved to the normal-alert neutral off red alert.
  const selected = page.locator('.stance-btn[aria-pressed="true"]');
  await expect(selected).toHaveCount(1);
  await expect(selected).toHaveClass(/default-row/);
});

test('command console: a human-held station\'s card is greyed with the CREWED cue, buttons disabled', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  const state = aiDirectedState({ directed_station_ai: false, default_selected: false });
  await page.evaluate((s) => window.__updateConsole('command', JSON.stringify(s)), state);

  await expect(page.locator('.station-cue-text').first()).toHaveText(ts('console.command.human_held'));
  // Criterion 3: greyed as a whole, not only its buttons.
  await expect(page.locator('.command-card').first()).toHaveClass(/crewed/);
  await expect(page.locator('.command-card .stance-btn').first()).toBeDisabled();
  await expect(page.locator('.command-card .stance-btn.default-row').first()).toBeDisabled();
});

test('command console: the Default row sends the resolved alert-neutral stance for the directed station', { tag: '@core' }, async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.evaluate((s) => window.__updateConsole('command', JSON.stringify(s)), aiDirectedState());

  await page.locator('.command-card .stance-btn.default-row').first().click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toMatchObject({
    action: 'set_station_stance',
    console: 'command',
    station: 'tactical',
    stance: 'tactical-normal',
  });
});

// Issue #1108 AC2: a human holding the Command-directed Station sees the current
// Command intent as NON-BINDING advice on their own console, and keeps full
// ordinary authority. The advice rides the target console's payload as
// `command_advice` (attached by `withCommandAdvice`); here it is fed directly to
// pin the render contract. Unaffected by #1381 — a different page entirely.
test('target console: shows Command intent as non-binding advice while human-held', async ({ page }) => {
  await page.goto('/gui/destroyer/tactical.html');
  const payload = {
    systems: {},
    own_hull: null,
    dossiers: [],
    command_advice: {
      stance_id: 'tactical-weapons-free',
      stance_label: 'entity.alliance_destroyer.station.tactical.stance.weapons_free',
      high_alert: true,
    },
  };
  await page.evaluate((s) => window.__updateConsole('tactical', JSON.stringify(s)), payload);

  const advice = page.locator('#command-advice');
  await expect(advice).toBeVisible();
  await expect(advice.locator('.advice-heading')).toHaveText(ts('console.command.advice_heading'));
  await expect(advice.locator('.advice-hint')).toHaveText(ts('console.command.advice_hint'));
  await expect(page.locator('#command-advice-stance')).toHaveText(
    ts('entity.alliance_destroyer.station.tactical.stance.weapons_free'),
  );

  // With no advice (the directed Station is AI, or this console is not the
  // target), the advisory line is hidden entirely.
  await page.evaluate(() => window.__updateConsole('tactical', JSON.stringify({ systems: {} })));
  await expect(page.locator('#command-advice')).toBeHidden();
});

// Issue #1109: an uncrewed Command seat is run by the ship AI, which selects an
// authored stance from the SAME catalogue a human uses. The console surfaces
// that with the `command_auto` cue and the AI-selected stance in force; a human
// taking the seat clears the cue and re-picks through the ordinary path. The
// selection logic itself is pinned in Rust (`operate_command_ai`, admission, the
// pure `select_stance`); this pins the console-boundary contract.
test('command console: an uncrewed Command shows the AI stance, and a human taking the seat can change it', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });

  // Uncrewed Command: the ship AI holds the seat and has selected the authored
  // engaged stance (weapons free) at Red Alert.
  const auto = aiDirectedState({ command_auto: true, selected_stance: 'tactical-weapons-free', default_selected: false });
  await page.evaluate((s) => window.__updateConsole('command', JSON.stringify(s)), auto);

  await expect(page.locator('.command-cue')).toBeVisible();
  await expect(page.locator('.command-cue-text')).toHaveText(ts('console.command.command_auto'));
  // The AI-selected intent (weapons free, an ordinary stance button) is the
  // marked stance in force.
  const aiSelected = page.locator('.stance-btn[aria-pressed="true"]');
  await expect(aiSelected).toHaveCount(1);
  await expect(aiSelected.locator('.name')).toHaveText(
    ts('entity.alliance_destroyer.station.tactical.stance.weapons_free'),
  );

  // A human takes the Command seat: the auto cue clears and they can re-pick.
  const crewed = aiDirectedState({ command_auto: false, selected_stance: 'tactical-weapons-free', default_selected: false });
  await page.evaluate((s) => window.__updateConsole('command', JSON.stringify(s)), crewed);
  await expect(page.locator('.command-cue')).toBeHidden();

  // Re-pick "hold" (the second standard stance) through the ordinary path.
  await page.locator('.command-card .stance-btn:not(.default-row)').nth(1).click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toMatchObject({
    action: 'set_station_stance',
    console: 'command',
    station: 'tactical',
    stance: 'tactical-hold',
  });
});

test('command console: clicking a stance emits set_station_stance for that station', { tag: '@core' }, async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.evaluate((s) => window.__updateConsole('command', JSON.stringify(s)), aiDirectedState());

  // Click the "weapons free" stance (first ordinary button).
  await page.locator('.command-card .stance-btn:not(.default-row)').first().click();

  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toMatchObject({
    action: 'set_station_stance',
    console: 'command',
    station: 'tactical',
    stance: 'tactical-weapons-free',
  });
});

// Issue #1110: an active scenario objective contributes an extra authored stance
// to the directed Station. The console is a pure projection of the resolved
// payload, so activation shows up as an added, selectable ordinary button, and
// when the objective ends the server drops it and moves `selected_stance` back
// to the alert-neutral — which now reads through the Default row (#1381)
// instead of as an authored-label button. The server-side contribution and
// removal are pinned in Rust (`ship::command_stance::effective_catalogue`, the
// command server systems, the objective manager); this pins the
// console-boundary contract.
test('command console: an objective stance appears and is selectable, then folds back into the Default row when the objective ends', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });

  // Objective active: the contributed stance joins the ordinary list as a
  // third option and is the stance in force. (Its label reuses an existing
  // string id — the console renders whatever label the blackboard carries.)
  const active = aiDirectedState({ default_selected: false });
  active.stations[0].stances.push({
    id: 'tactical-objective-escort',
    label: 'entity.alliance_destroyer.station.tactical.stance.weapons_free',
    kind: 'standard',
    high_alert: true,
  });
  active.stations[0].selected_stance = 'tactical-objective-escort';
  await page.evaluate((s) => window.__updateConsole('command', JSON.stringify(s)), active);

  await expect(page.locator('.command-card .stance-btn:not(.default-row)')).toHaveCount(3);
  await expect(page.locator('.command-card .stance-btn.default-row')).toHaveCount(1);
  await expect(page.locator('.stance-btn[aria-pressed="true"]')).toHaveCount(1);

  // It is selectable: clicking it emits set_station_stance for the objective id.
  await page.locator('.command-card .stance-btn:not(.default-row)').nth(2).click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toMatchObject({
    action: 'set_station_stance',
    console: 'command',
    station: 'tactical',
    stance: 'tactical-objective-escort',
  });

  // Objective ends: the server withdraws the option and moves selected_stance
  // back to the alert-neutral. The extra button disappears, and the row now
  // marked in force is the Default row — not an ordinary button.
  const ended = aiDirectedState(); // the two permanent standard stances; Default row in force
  await page.evaluate((s) => window.__updateConsole('command', JSON.stringify(s)), ended);

  await expect(page.locator('.command-card .stance-btn:not(.default-row)')).toHaveCount(2);
  const fellBack = page.locator('.stance-btn[aria-pressed="true"]');
  await expect(fellBack).toHaveCount(1);
  await expect(fellBack).toHaveClass(/default-row/);
  await expect(fellBack.locator('.name')).toHaveText(ts('console.command.stance.default'));
});
