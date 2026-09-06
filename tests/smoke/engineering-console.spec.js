import { test, expect } from './fixtures';

// ── Shields over Power / Repair / Operations layout (issue #1383) ──────────
// The destroyer's Engineering seat is the one console with three system
// families to arrange (Shields, Power, Repair) plus a fourth grouping
// (Operations: Tractor, Umbilical) that only a phone needs to segment away —
// desktop/landscape show all three columns at once. Mirrors
// `gui/destroyer/captain.html`'s TARGET | MISSION coverage in
// tests/smoke/captain-console.spec.js (issue #1377).
//
// This exercises the same separately-built Trunk host bundle
// tests/smoke/captain-console.spec.js's destroyer cases do (`trunk build`,
// not `scripts/build-client.mjs`) — see that file's own note on `/gui/...`
// vs `/client/gui/...`.
const ENGINEERING_URL = '/gui/destroyer/engineering.html';

function destroyerEngineeringState() {
  return {
    system_ids: ['shields-system', 'power-reactor', 'repair'],
    system_families: {
      'shields-system': 'shields',
      'power-reactor': 'power',
      repair: 'repair',
    },
    systems: {
      'shields-system': {
        facings: [{ id: 'fwd' }],
        focused_facing: 'fwd',
        shields_auto: true,
        threat_bearing: null,
      },
      'power-reactor': {
        consoles: [{ id: 'helm', level: 2, commanded_level: 2, min_level: 1, max_level: 4 }],
        power_auto: true,
        battery_online: true,
        charging: false,
        battery_charge: 60,
        battery_max: 100,
      },
      repair: {
        overall_hull: { pct: 1, destroyed_pct: 0 },
        core_systems: [],
        teams: [],
        repair_auto: true,
        dispatch_targets: [],
        damaged_systems: [],
      },
    },
    own_hull: { pct: 1 },
  };
}

async function pushDestroyerEngineeringState(page) {
  await page.waitForFunction(() => typeof window.__updateConsole === 'function');
  await page.evaluate((s) => window.__updateConsole('engineering', JSON.stringify(s)), destroyerEngineeringState());
}

test('destroyer engineering console: the phone segment switches REPAIR and OPS and keeps one tab stop', { tag: '@core' }, async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(ENGINEERING_URL);
  await pushDestroyerEngineeringState(page);

  const repairTab = page.locator('#engineering-seg-repair');
  const opsTab = page.locator('#engineering-seg-ops');
  const repairPanel = page.locator('#engineering-panel-repair');
  const opsPanel = page.locator('#engineering-panel-ops');

  // Default: REPAIR selected and visible, OPS hidden and out of the tab order.
  await expect(repairTab).toHaveAttribute('aria-selected', 'true');
  await expect(opsTab).toHaveAttribute('aria-selected', 'false');
  await expect(repairPanel).toBeVisible();
  await expect(opsPanel).toBeHidden();
  expect(await opsTab.getAttribute('tabindex')).toBe('-1');

  await opsTab.click();

  await expect(opsTab).toHaveAttribute('aria-selected', 'true');
  await expect(repairTab).toHaveAttribute('aria-selected', 'false');
  await expect(opsPanel).toBeVisible();
  await expect(repairPanel).toBeHidden();
  expect(await repairTab.getAttribute('tabindex')).toBe('-1');
  expect(await opsTab.getAttribute('tabindex')).toBe('0');

  // Arrow-key roving moves the single tab stop AND re-selects (the same
  // automatic activation the Station Bar's own tabs use).
  await opsTab.focus();
  await page.keyboard.press('ArrowLeft');
  await expect(repairTab).toBeFocused();
  await expect(repairTab).toHaveAttribute('aria-selected', 'true');
  await expect(repairPanel).toBeVisible();
  await expect(opsPanel).toBeHidden();
});

test('destroyer engineering console: Repair and Operations both stay on screen at desktop width', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(ENGINEERING_URL);
  await pushDestroyerEngineeringState(page);

  await expect(page.locator('#engineering-seg')).toBeHidden();
  await expect(page.locator('#engineering-panel-repair')).toBeVisible();
  await expect(page.locator('#engineering-panel-ops')).toBeVisible();
});

test('destroyer engineering console: at 390px neither the console body nor the page needs to scroll', { tag: '@core' }, async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(ENGINEERING_URL);
  await pushDestroyerEngineeringState(page);

  const overflow = await page.evaluate(() => {
    const body = document.querySelector('.console-body');
    return {
      bodyScrolls: body.scrollHeight > body.clientHeight + 1,
      pageScrolls: document.documentElement.scrollHeight > window.innerHeight + 1,
    };
  });
  expect(overflow.bodyScrolls).toBe(false);
  expect(overflow.pageScrolls).toBe(false);
});
