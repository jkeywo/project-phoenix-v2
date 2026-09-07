import { test, expect } from './fixtures';

const CONSOLE_URL = '/gui/battleship/repair.html';

// The cruiser has no dedicated Repair console: its `repair` System hangs off
// the Engineering seat, so the Field Repair row this file's cruiser case
// exercises lives on that console instead (issue #1391). Everything else on
// this page is the same shared `ph-repair-teams` / `ph-hull-integrity` the
// battleship's console mounts.
const CRUISER_ENGINEERING_URL = '/gui/cruiser/engineering.html';

function repairState(overrides = {}) {
  return Object.assign(
    {
      teams: [
        { id: 0, label: 'Team 1', status: 'idle', target: '', progress_pct: 0 },
        { id: 1, label: 'Team 2', status: 'idle', target: '', progress_pct: 0 },
      ],
      dispatch_targets: [
        { id: 'helm', label: 'Helm', damage_pct: 0.2 },
        { id: 'tactical', label: 'Tactical', damage_pct: 0.5 },
        { id: 'core', label: 'Core', damage_pct: 0.1 },
      ],
      core_systems: [
        { system_id: 'core', display_name: 'Core', current: 18.0, max_hp: 20.0 },
      ],
      overall_hull: { current: 80, max: 100, pct: 0.8 },
      travel_duration_secs: 5.0,
      repair_auto: false,
      // Issue #1015: the tap-to-prioritise list, re-homed by #1384 into the
      // on-site team's own card. Worst-first, as the host folds it;
      // `prioritised` is the host's own resolved pin, and `prioritisable` is
      // projected from the owner's candidate predicate — the component treats
      // it as the SOLE enablement fact, so a row without it renders disabled
      // and its tap resolves to nothing.
      damaged_systems: [
        { system_id: 'aux-sensor', display_name: 'Auxiliary Sensor', tier: 'Destroyed', current: 0, max_hp: 10, damage_pct: 1.0, prioritised: false, in_progress: false, prioritisable: true },
        { system_id: 'hull-plating', display_name: 'Hull Plating', tier: 'Disabled', current: 2, max_hp: 20, damage_pct: 0.9, prioritised: false, in_progress: true, prioritisable: false },
        { system_id: 'core', display_name: 'Core', tier: 'Damaged', current: 12, max_hp: 20, damage_pct: 0.4, prioritised: true, in_progress: false, prioritisable: true },
      ],
      // #1287 moved dispatch and prioritise onto shared semantic actions, whose
      // handlers resolve a control_system_id from this projection and REFUSE
      // the action without it — a payload missing it sends nothing at all.
      systems: { damage_control: {} },
      system_families: { damage_control: 'repair' },
    },
    overrides,
  );
}

/** One on-site team, so the card that carries the damaged list can be opened. */
function onSiteState(overrides = {}) {
  return repairState(Object.assign({
    teams: [
      { id: 0, label: 'Team 1', status: 'repairing', target: 'Helm', progress_pct: 1 },
      { id: 1, label: 'Team 2', status: 'idle', target: '', progress_pct: 0 },
    ],
  }, overrides));
}

/** Open one team's card the way a player does (issue #1384). */
const selectTeam = (page, id) =>
  page.locator(`ph-repair-teams .card[data-team-id="${id}"] .card-top`).click();

test('repair console: renders overall hull and the collapsed team roster', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState());

  const fill = page.locator('ph-hull-integrity ph-damage-bar').locator('#bar-fill');
  const width = await fill.evaluate((el) => el.style.width);
  expect(width).toBe('80%');
  await expect(page.locator('ph-repair-teams .card')).toHaveCount(2);
  // Nothing is dispatchable until a team is selected: the roster is a summary.
  await expect(page.locator('ph-repair-teams .btn')).toHaveCount(0);
  await selectTeam(page, 0);
  await expect(page.locator('ph-repair-teams .card[data-team-id="0"] .btn')).toHaveCount(3);
  await expect(page.locator('ph-repair-teams .card[data-team-id="1"] .btn')).toHaveCount(0);
});

test('repair console: Core bar hides when there are no damageable core systems', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState({ core_systems: [] }));
  await expect(page.locator('#core-damage')).toBeHidden();
});

test('repair console: Core bar shows and pops up damaged core systems when clicked', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState());
  const coreBar = page.locator('#core-damage');
  await expect(coreBar).toBeVisible();
  await coreBar.locator('.bar').click();
  await expect(coreBar.locator('.popup')).toHaveClass(/open/);
  await expect(coreBar.locator('ph-damage-detail .row')).toHaveCount(1);
});

test('repair console: dispatch buttons call __sendAction with correct envelope', { tag: '@core' }, async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState());
  await selectTeam(page, 0);
  await page.locator('ph-repair-teams .card[data-team-id="0"] .btn[data-target="helm"]').click();
  await selectTeam(page, 1);
  await page.locator('ph-repair-teams .card[data-team-id="1"] .btn[data-target="tactical"]').click();

  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(2);
  expect(JSON.parse(sent[0])).toMatchObject({ action: 'dispatch_repair_team', console: 'repair', team_idx: 0, target: 'helm' });
  expect(JSON.parse(sent[1])).toMatchObject({ action: 'dispatch_repair_team', console: 'repair', team_idx: 1, target: 'tactical' });
});

test('repair console: selecting a second team closes the first card', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), repairState());
  await selectTeam(page, 0);
  await selectTeam(page, 1);
  await expect(page.locator('ph-repair-teams .card[data-team-id="0"] .card-top'))
    .toHaveAttribute('aria-expanded', 'false');
  await expect(page.locator('ph-repair-teams .card[data-team-id="0"] .btn')).toHaveCount(0);
  await expect(page.locator('ph-repair-teams .card[data-team-id="1"] .card-top'))
    .toHaveAttribute('aria-expanded', 'true');
});

// Issue #1386: the field target is a destination like any other, and the seat
// says WHICH team crosses over. The @core breadth test for per-team field
// dispatch: this is the whole feature in one pass — choose a team, send it off
// the ship, and get a command naming that team.
test('repair console: an idle card sends a NAMED dispatch to the field target', { tag: '@core' }, async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.evaluate(
    (s) => window.__updateConsole('repair', JSON.stringify(s)),
    repairState({
      external_dispatch: {
      candidate_name: 'world.probe_external_repair.entity.ally.name',
        range: 800, target: null, target_name: null, refusal: null, team_idx: null,
      },
    }),
  );
  await selectTeam(page, 1);
  // The three ship destinations plus the field target.
  await expect(page.locator('ph-repair-teams .card[data-team-id="1"] .btn')).toHaveCount(4);
  await page.locator('ph-repair-teams .card[data-team-id="1"] .field-btn').click();

  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  // The same verb the station buttons beside it send, naming THIS card's team.
  // Only the destination is the host's to resolve — it is Tactical's lock — so
  // `external` carries no id of its own (#1386).
  expect(JSON.parse(sent[0])).toMatchObject({
    action: 'dispatch_repair_team', console: 'repair', team_idx: 1, target: 'external',
  });
});

// The other half of the same claim: once a team IS abroad its card says so, and
// the ONE recall verb brings it home. Its wire slot is still `Idle` — it never
// walked anywhere on this hull — so everything here is derived from the claim
// the host published, not from the slot.
test('repair console: the abroad card shows the target it works and recalls it', { tag: '@core' }, async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.evaluate(
    (s) => window.__updateConsole('repair', JSON.stringify(s)),
    repairState({
      external_dispatch: {
      candidate_name: 'world.probe_external_repair.entity.ally.name',
        range: 800,
        target: 'uuid-ally',
        target_name: 'console.repair.dispatch',
        refusal: null,
        team_idx: 1,
        target_condition: 0.35,
      },
    }),
  );

  const abroad = page.locator('ph-repair-teams .card[data-team-id="1"]');
  await expect(abroad.locator('.status-badge')).toHaveText('ABROAD');
  // The bar is the TARGET's condition track: a team abroad is already there, so
  // its own travel/repair progress would be nothing to look at.
  await expect(abroad.locator('.progress-fill')).toHaveClass(/abroad/);
  expect(await abroad.locator('.progress-fill').evaluate((el) => el.style.width)).toBe('35%');

  // The other idle team may not be sent while the claim is held, and is told why.
  await selectTeam(page, 0);
  await expect(page.locator('ph-repair-teams .card[data-team-id="0"] .field-btn')).toBeDisabled();

  await selectTeam(page, 1);
  await abroad.locator('.recall-btn').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toMatchObject({
    action: 'recall_repair_team', console: 'repair', team_idx: 1,
  });
});

test('repair console: the on-site card lists its systems worst-first and highlights the host pin', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), onSiteState());

  // Nothing until the on-site team's card is open: the standalone list is gone.
  await expect(page.locator('ph-repair-teams .dmg-row')).toHaveCount(0);
  await selectTeam(page, 0);

  const rows = page.locator('ph-repair-teams .card[data-team-id="0"] .dmg-row');
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toHaveAttribute('data-system-id', 'aux-sensor');
  await expect(rows.nth(2)).toHaveAttribute('data-system-id', 'core');
  // Exactly one highlight, and it is the row the HOST pinned — not the worst
  // row, which is how you can tell nothing here re-derived the choice.
  await expect(page.locator('ph-repair-teams .dmg-row.prioritised')).toHaveCount(1);
  await expect(page.locator('ph-repair-teams .dmg-row.prioritised')).toHaveAttribute('data-system-id', 'core');
  // The per-team 1/2/3 ordinal buttons this list replaced are gone.
  await expect(page.locator('ph-repair-teams .priority-btn')).toHaveCount(0);
});

test('repair console: tapping a system in the on-site card sends set_repair_target_priority', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), onSiteState());
  await selectTeam(page, 0);
  await page.locator('ph-repair-teams .dmg-row[data-system-id="aux-sensor"]').click();

  const sent = await page.evaluate(() => window.__sent);
  // No team index and no ordinal: the host resolves which team and pins the
  // system; the ordinal is untouched.
  expect(sent).toHaveLength(1);
  expect(JSON.parse(sent[0])).toMatchObject({
    action: 'set_repair_target_priority', console: 'repair', system_id: 'aux-sensor',
  });
});

test('repair console: a row a team is already on is shown but not offered', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), onSiteState());
  await selectTeam(page, 0);

  // `hull-plating` is the in_progress row. A tap on it is structurally a no-op —
  // the host's sweep never offers the system a team is standing on as a
  // candidate — so it is rendered (the damage is real) but not offered as a
  // control. `core`, which the host pinned, stays live.
  await expect(page.locator('ph-repair-teams .dmg-row[data-system-id="hull-plating"]'))
    .toBeDisabled();
  await expect(page.locator('ph-repair-teams .dmg-row[data-system-id="core"]'))
    .toBeEnabled();
});

// Issue #1385: an on-site team with no visible rows still has one verb, so its
// card opens onto RECALL alone. A RETURNING team is the case with no body at
// all — the host refuses a recall for a team already walking home — and that is
// where the "a summary with nothing behind it is a readout" rule now lives.
test('repair console: an on-site card with no visible rows opens onto RECALL alone', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate((s) => window.__updateConsole('repair', JSON.stringify(s)), onSiteState({ damaged_systems: [] }));
  const card = page.locator('ph-repair-teams .card[data-team-id="0"]');
  await card.locator('.card-top').click();
  await expect(card.locator('.card-body')).toBeVisible();
  await expect(page.locator('ph-repair-teams .dmg-row')).toHaveCount(0);
  // Zero rows is not the same claim as "the section went away": an empty
  // [DAMAGED SYSTEMS] heading over nothing would pass a count-only assertion.
  await expect(card.locator('.body-head')).toHaveText('');
  await expect(card.locator('.recall-btn')).toHaveCount(1);
});

test('repair console: a returning team is not offered as a toggle at all', async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(
    (s) => window.__updateConsole('repair', JSON.stringify(s)),
    repairState({
      teams: [
        { id: 0, label: 'Team 1', status: 'returning', target: 'Helm', progress_pct: 0.9 },
        { id: 1, label: 'Team 2', status: 'idle', target: '', progress_pct: 0 },
      ],
    }),
  );
  const top = page.locator('ph-repair-teams .card[data-team-id="0"] .card-top');
  // Not merely "opens onto nothing": a summary with no body behind it is a
  // readout, so the tap never happens and the chrome never claims it did.
  await expect(top).toBeDisabled();
  await expect(top).toHaveAttribute('aria-expanded', 'false');
  await expect(page.locator('ph-repair-teams .card[data-team-id="0"] .card-body')).toBeHidden();
});

test('repair console: RECALL on a team out on a job sends recall_repair_team', { tag: '@core' }, async ({ page }) => {
  await page.goto(CONSOLE_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = (json) => window.__sent.push(json);
  });
  await page.evaluate(
    (s) => window.__updateConsole('repair', JSON.stringify(s)),
    repairState({
      teams: [
        { id: 0, label: 'Team 1', status: 'travelling', target: 'Helm', progress_pct: 0.3 },
        { id: 1, label: 'Team 2', status: 'idle', target: '', progress_pct: 0 },
      ],
    }),
  );
  // Nothing until the card is open: the roster is a summary (issue #1384).
  await expect(page.locator('ph-repair-teams .recall-btn')).toHaveCount(0);
  await selectTeam(page, 0);
  const card = page.locator('ph-repair-teams .card[data-team-id="0"]');
  // A travelling team has exactly one verb, and no destinations to confuse it
  // with: RECALL is not one more place to send it.
  await expect(card.locator('.btn')).toHaveCount(1);
  await card.locator('.recall-btn').click();

  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toHaveLength(1);
  // Names its team, unlike the fieldless external recall: a hull can have every
  // internal team out on a different job at once.
  expect(JSON.parse(sent[0])).toMatchObject({
    action: 'recall_repair_team', console: 'repair', team_idx: 0,
  });
});

// Cruiser field work uses the selected repair card and the named-team verb.
function cruiserEngineeringState(externalDispatch) {
  return {
    system_ids: ['power-reactor', 'repair'],
    system_families: { 'power-reactor': 'power', repair: 'repair' },
    systems: {
      'power-reactor': { consoles: [], power_auto: false },
      repair: {
        teams: [0, 1].map(id => ({ id, label: `Team ${id + 1}`, status: 'idle', progress_pct: 0 })),
        dispatch_targets: [], damaged_systems: [], core_systems: [],
        overall_hull: { current: 90, max: 100, pct: 0.9 },
        repair_auto: false, external_dispatch: externalDispatch,
      },
    },
  };
}

const fieldTargetName = 'world.probe_external_repair.entity.ally.name';
test('repair console: cruiser Team 2 dispatches to the named field target and recalls', { tag: '@core' }, async ({ page }) => {
  await page.goto(CRUISER_ENGINEERING_URL);
  await page.evaluate(() => {
    window.__sent = [];
    window.__sendAction = json => window.__sent.push(JSON.parse(json));
  });
  const publish = async external => page.evaluate(
    s => window.__updateConsole('engineering', JSON.stringify(s)),
    cruiserEngineeringState(external),
  );
  await publish({ range: 400, target: null, candidate_name: null });
  const team = page.locator('ph-repair-teams .card[data-team-id="1"]');
  await team.locator('.card-top').click();
  await expect(team.locator('.field-btn')).toHaveCount(0);
  await publish({ range: 400, target: null, candidate_name: fieldTargetName,
    candidate_refusal: 'repair.dispatch.refused.out_of_range' });
  const field = team.locator('.field-btn');
  await expect(field).toBeDisabled();
  const expected = await page.evaluate(async id => (await import('/gui/strings.js')).t(id), fieldTargetName);
  await expect(field).toContainText(expected);
  await publish({ range: 400, target: null, candidate_name: fieldTargetName });
  await expect(field).toBeEnabled();
  await field.focus();
  await page.keyboard.press('Enter');
  await publish({ range: 400, target: 'ally-uuid', target_name: fieldTargetName,
    team_idx: 1, target_condition: 0.42, candidate_name: fieldTargetName });
  await expect(team).toContainText(expected);
  await expect(team.locator('.progress-fill')).toHaveAttribute('style', /42%/);
  await team.locator('.recall-btn').click();
  const sent = await page.evaluate(() => window.__sent);
  expect(sent).toEqual([
    expect.objectContaining({ action: 'dispatch_repair_team', console: 'engineering',
      control_system_id: 'repair', team_idx: 1, target: 'external' }),
    expect.objectContaining({ action: 'recall_repair_team', console: 'engineering',
      control_system_id: 'repair', team_idx: 1 }),
  ]);
});
