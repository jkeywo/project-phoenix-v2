import { test, expect } from '@playwright/test';

// A populated available-berth label is taller than Docked. The contextual
// control must fit the real console iframe after the shell takes its bar.
const viewports = [
  ['phone-375x760', 375, 760],
  ['phone-390x792', 390, 792],
  ['landscape-716x375', 716, 375],
  ['desktop-1280x900', 1280, 900],
];
const modes = {
  available: { system_id: 'cruiser-docking-clamps', available: true, engaged: false, docked: false, available_target_name: 'world.probe_dock.entity.berth.name' },
  docked: { system_id: 'cruiser-docking-clamps', available: false, engaged: true, docked: true, docked_to_name: 'world.probe_dock.entity.berth.name' },
};
const base = {
  system_ids: ['cruiser-docking-clamps'],
  system_families: { 'cruiser-docking-clamps': 'helm' },
  blips: [
    { id: 'berth', name: 'HALDEN YARD', x: 80, z: 45, kind: 'station', friendly: true },
    { id: 'escort', name: 'KESTREL', x: -125, z: -70, kind: 'ship', friendly: true },
  ],
  range: 500, x: 1240, z: -380, ship_heading: 47, speed: 12.4, on_screen: true,
  engine_port_thrust: 0.35, engine_stbd_thrust: 0.62, hostile_arcs: [],
  helm_auto: false, lateral_auto: false, impulse_charge_progress: 0,
  boost_enabled: true, boost_active: false, boost_battery: 0.82, own_hull: { pct: 0.94 },
};

for (const [label, width, height] of viewports) {
  for (const [mode, dock] of Object.entries(modes)) {
    test(`Cruiser Helm ${label} ${mode} fits its viewport`, async ({ page }) => {
      await page.setViewportSize({ width, height });
      await page.goto('/gui/cruiser/helm.html');
      await page.waitForFunction(() => typeof window.__updateConsole === 'function' && document.querySelector('ph-helm-radar')?.shadowRoot);
      await page.evaluate(payload => window.__updateConsole('helm', JSON.stringify(payload)), { ...base, dock });
      await page.evaluate(() => document.fonts.ready);
    const measurement = await page.evaluate(() => {
      const b = (el) => { const r = el.getBoundingClientRect(); return { x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom }; };
      const ids = ['dock-btn','dock-panel','helm-radar','helm-joystick','lateral-thrust-joystick','impulse-btn','boost-btn'];
      const boxes = Object.fromEntries(ids.map(id => [id, b(document.getElementById(id))]));
      const body = document.querySelector('.console-body');
      const scope = document.querySelector('.scope-cell');
      const children = [...body.children].map(el => ({ cls: el.className, ...b(el), scrollHeight: el.scrollHeight, clientHeight: el.clientHeight }));
      const controls = ids.map(id => document.getElementById(id)).filter(Boolean);
      const outerClipping = controls.filter(el => { const r=b(el); return r.x < -1 || r.y < -1 || r.right > innerWidth+1 || r.bottom > innerHeight+1; }).map(el => el.id);
      const overlap = (a,c) => Math.min(a.right,c.right)-Math.max(a.x,c.x)>1 && Math.min(a.bottom,c.bottom)-Math.max(a.y,c.y)>1;
      const scopeBox=b(scope);
      const scopeIntruders=controls.filter(el => !scope.contains(el) && overlap(b(el),scopeBox)).map(el=>el.id);
      return { innerWidth, innerHeight, boxes, body: b(body), bodyScrollHeight: body.scrollHeight, bodyClientHeight: body.clientHeight, bodyOverflowY:getComputedStyle(body).overflowY, children, outerClipping, scopeIntruders, labels:{dock:document.getElementById('dock-btn').textContent,status:document.getElementById('dock-status').textContent} };
    });
      expect(measurement.outerClipping).toEqual([]);
      expect(measurement.scopeIntruders).toEqual([]);
      expect(measurement.bodyScrollHeight).toBeLessThanOrEqual(measurement.bodyClientHeight + 1);
      expect(measurement.boxes['dock-btn'].height).toBeGreaterThanOrEqual(44);
      expect(measurement.boxes['dock-btn'].width).toBeGreaterThanOrEqual(44);
      const scope = measurement.boxes['helm-radar'];
      expect(Math.abs(scope.width - scope.height)).toBeLessThanOrEqual(1);
      expect(scope.width).toBeGreaterThanOrEqual(Math.min(width, height) / 3);
      expect(measurement.boxes['helm-joystick'].width).toBeGreaterThanOrEqual(130);
    });
  }
}
