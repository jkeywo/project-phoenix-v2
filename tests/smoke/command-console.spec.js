import { test, expect } from './fixtures';
import { ts } from './strings';

// End-to-end at the client boundary (issue #1107): the auxiliary Command
// console, driven by the `command` blackboard a server carrying one
// AI-controlled proving Station publishes. The stance selection round-trip and
// the server-side application are pinned in the Rust admission/command tests and
// the pure `ship::command_stance` resolver; this spec pins the console's own
// contract — it lists the directed Station's stances only while that Station is
// AI-controlled, shows a persistent non-colour automation cue, marks the stance
// in force, and emits the `set_station_stance` order.

const CONSOLE_URL = '/gui/command-console.html';

function aiDirectedState() {
  return {
    command_system_id: 'command',
    directed_station: 'tactical',
    directed_station_name: 'Tactical',
    directed_station_ai: true,
    command_auto: false,
    selected_stance: 'tactical-normal',
    stances: [
      { id: 'tactical-weapons-free', label: 'entity.alliance_destroyer.station.tactical.stance.weapons_free', kind: 'standard', high_alert: true },
      { id: 'tactical-hold', label: 'entity.alliance_destroyer.station.tactical.stance.hold', kind: 'standard', high_alert: false },
      { id: 'tactical-normal', label: 'entity.alliance_destroyer.station.tactical.stance.normal', kind: 'normal_alert_neutral', high_alert: false },
      { id: 'tactical-high', label: 'entity.alliance_destroyer.station.tactical.stance.high', kind: 'high_alert_neutral', high_alert: true },
    ],
  };
}

test('command console: lists the AI-controlled station stances with a directable cue', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('command', JSON.stringify(s)), aiDirectedState());

  await expect(page.locator('#directed-name')).toHaveText('Tactical');
  // One button per authored stance.
  await expect(page.locator('#stance-list .stance-btn')).toHaveCount(4);
  // Persistent non-colour cue: the directed station is AI and therefore
  // directable, so the buttons are enabled.
  await expect(page.locator('#station-cue-text')).toHaveText(ts('console.command.ai_directed'));
  await expect(page.locator('#stance-list .stance-btn').first()).toBeEnabled();
  // The stance in force is marked (a glyph, not a colour).
  const selected = page.locator('.stance-btn[aria-pressed="true"]');
  await expect(selected).toHaveCount(1);
});

test('command console: a human-held station is off the board (buttons disabled)', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  const state = aiDirectedState();
  state.directed_station_ai = false;
  await page.evaluate((s) => window.__updateConsole('command', JSON.stringify(s)), state);

  await expect(page.locator('#station-cue-text')).toHaveText(ts('console.command.human_held'));
  await expect(page.locator('#stance-list .stance-btn').first()).toBeDisabled();
});

test('command console: clicking a stance emits set_station_stance for that station', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.evaluate((s) => window.__updateConsole('command', JSON.stringify(s)), aiDirectedState());

  // Click the "weapons free" stance (first button).
  await page.locator('#stance-list .stance-btn').first().click();

  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toEqual({
    action: 'set_station_stance',
    console: 'command',
    station: 'tactical',
    stance: 'tactical-weapons-free',
  });
});
