import { test, expect } from '@playwright/test';

const viewports = [
  ['phone-375x760', 375, 760],
  ['phone-390x792', 390, 792],
  ['landscape-716x375', 716, 375],
  ['desktop-1308x900', 1308, 900],
];

const payload = {
  system_ids: ['power-reactor', 'power-battery', 'repair', 'tractor', 'umbilical'],
  system_families: {
    'power-reactor': 'power', 'power-battery': 'power', repair: 'repair',
    tractor: 'tractor', umbilical: 'umbilical',
  },
  systems: {
    'power-reactor': {
      consoles: [
        { id: 'propulsion', label: 'PROPULSION', level: 2, commanded_level: 2, min_level: 1, max_level: 4 },
        { id: 'weapons', label: 'WEAPONS', level: 3, commanded_level: 3, min_level: 0, max_level: 4 },
        { id: 'shields', label: 'SHIELDS', level: 2, commanded_level: 2, min_level: 1, max_level: 4 },
      ],
      power_auto: false, battery_online: true, charging: true,
      battery_charge: 72, battery_max: 100,
    },
    repair: {
      overall_hull: { pct: 0.84, destroyed_pct: 0.08 },
      core_systems: [{ system_id: 'reactor-core', label: 'Reactor Core', tier: 'Damaged', damage_pct: 0.22 }],
      repair_auto: false,
      teams: [
        { id: 0, label: 'Team 1', status: 'repairing', target: 'Tactical', progress_pct: 0.45 },
        { id: 1, label: 'Team 2', status: 'idle', target: null, progress_pct: 0 },
      ],
      dispatch_targets: [{ id: 'helm', label: 'Helm' }, { id: 'tactical', label: 'Tactical' }],
      damaged_systems: [
        { system_id: 'port-blaster', label: 'Port Blaster', owner: 'tactical', tier: 'Disabled', damage_pct: 0.82, prioritisable: true },
        { system_id: 'aft-sensors', label: 'Aft Sensor Cluster', owner: 'tactical', tier: 'Damaged', damage_pct: 0.26, prioritisable: true },
      ],
      external_dispatch: { range: 400, target: null, candidate_name: null },
    },
    tractor: { system_id: 'tractor', engaged: true, coupled_target_name: 'ALDRIC', range: 500 },
    umbilical: { system_id: 'umbilical', running: true, rate: 20, operator_level: 80, partner_level: 20 },
  },
  own_hull: { pct: 0.84 },
};

const fieldTargetName = 'world.probe_external_repair.entity.ally.name';

for (const [label, width, height] of viewports) {
  test(`Cruiser Engineering ${label} populated field-repair controls remain reachable`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await page.goto('/gui/cruiser/engineering.html');
    await page.waitForFunction(() => typeof window.__updateConsole === 'function' && document.querySelector('ph-repair-teams')?.shadowRoot);
    await page.evaluate(state => window.__updateConsole('engineering', JSON.stringify(state)), payload);
    await page.evaluate(() => document.fonts.ready);

    const team = page.locator('ph-repair-teams .card[data-team-id="1"]');
    await team.locator('.card-top').click();
    await expect(team.locator('.field-btn')).toHaveCount(0);

    const publishField = external_dispatch => page.evaluate(({ state, external_dispatch }) => {
      const next = structuredClone(state);
      next.systems.repair.external_dispatch = external_dispatch;
      window.__updateConsole('engineering', JSON.stringify(next));
    }, { state: payload, external_dispatch });

    await publishField({ range: 400, target: null, candidate_name: fieldTargetName });
    const field = team.locator('.field-btn');
    const expectedTarget = await page.evaluate(async id => (await import('/gui/strings.js')).t(id), fieldTargetName);
    const expectedRefusal = await page.evaluate(async id => (await import('/gui/strings.js')).t(id), 'repair.dispatch.refused.out_of_range');
    await expect(field).toBeEnabled();
    await expect(field).toContainText(expectedTarget);
    await field.scrollIntoViewIfNeeded();
    let fieldMetrics = await field.evaluate(el => ({
      height: el.getBoundingClientRect().height,
      scrollWidth: el.scrollWidth, clientWidth: el.clientWidth,
    }));
    expect(fieldMetrics.height).toBeGreaterThanOrEqual(44);
    expect(fieldMetrics.scrollWidth).toBeLessThanOrEqual(fieldMetrics.clientWidth + 1);

    await publishField({ range: 400, target: null, candidate_name: fieldTargetName,
      candidate_refusal: 'repair.dispatch.refused.out_of_range' });
    await expect(field).toBeDisabled();
    await expect(field).toHaveAttribute('title', expectedRefusal);

    await publishField({ range: 400, target: 'ally-uuid', target_name: fieldTargetName,
      team_idx: 1, target_condition: 0.42, candidate_name: fieldTargetName });
    await expect(team).toContainText(expectedTarget);
    await expect(team.locator('.progress-fill')).toHaveAttribute('style', /42%/);
    const recall = team.locator('.recall-btn');
    await recall.scrollIntoViewIfNeeded();
    const recallMetrics = await recall.evaluate(el => ({
      height: el.getBoundingClientRect().height,
      scrollWidth: el.scrollWidth, clientWidth: el.clientWidth,
    }));
    expect(recallMetrics.height).toBeGreaterThanOrEqual(44);
    expect(recallMetrics.scrollWidth).toBeLessThanOrEqual(recallMetrics.clientWidth + 1);

    const measurement = await page.evaluate(() => {
      const box = el => { const r = el.getBoundingClientRect(); return { x:r.x, y:r.y, right:r.right, bottom:r.bottom, width:r.width, height:r.height }; };
      const repair = document.querySelector('.repair-col');
      const fixedButtons = [...document.querySelector('ph-power-controls').shadowRoot.querySelectorAll('button')];
      const outsidePage = fixedButtons.filter(el => { const r=box(el); return r.x < -1 || r.right > innerWidth + 1 || r.y < -1 || r.bottom > innerHeight + 1; }).map(el => el.id || el.className);
      return {
        body: box(document.querySelector('.console-body')),
        bodyScrollHeight: document.querySelector('.console-body').scrollHeight,
        bodyClientHeight: document.querySelector('.console-body').clientHeight,
        repairScrollHeight: repair.scrollHeight,
        repairClientHeight: repair.clientHeight,
        repairOverflow: getComputedStyle(repair).overflowY,
        outsidePage,
        tractor: box(document.getElementById('tractor-btn')),
        umbilical: box(document.getElementById('umbilical-btn')),
        visiblePanels: ['tractor-panel','umbilical-panel'].filter(id => !document.getElementById(id).hidden),
      };
    });

    expect(measurement.body.bottom).toBeLessThanOrEqual(height + 1);
    expect(measurement.bodyScrollHeight).toBeLessThanOrEqual(measurement.bodyClientHeight + 1);
    expect(measurement.repairOverflow).toBe('auto');
    expect(measurement.outsidePage).toEqual([]);
    expect(measurement.visiblePanels).toEqual(['tractor-panel', 'umbilical-panel']);

    for (const id of ['tractor-btn', 'umbilical-btn']) {
      await page.locator(`#${id}`).scrollIntoViewIfNeeded();
      const reached = await page.evaluate(buttonId => {
        const button = document.getElementById(buttonId).getBoundingClientRect();
        const repair = document.querySelector('.repair-col').getBoundingClientRect();
        return { height: button.height, top: button.top, bottom: button.bottom, repairTop: repair.top, repairBottom: repair.bottom };
      }, id);
      expect(reached.height).toBeGreaterThanOrEqual(44);
      expect(reached.top).toBeGreaterThanOrEqual(reached.repairTop - 1);
      expect(reached.bottom).toBeLessThanOrEqual(reached.repairBottom + 1);
    }
    await page.screenshot({ path: `target/console-redesign-resume/1391/responsive-${label}.png` });
  });
}
