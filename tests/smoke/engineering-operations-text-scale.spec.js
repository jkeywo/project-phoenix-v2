import { test, expect } from '@playwright/test';
import {
  DEVICE_MATRIX,
  TEXT_SCALES,
  BROWSER_ZOOMS,
} from '../fixtures/device-matrix.mjs';

/**
 * tests/smoke/engineering-operations-text-scale.spec.js — issue #1425 (PRD
 * #1418 stories 1, 2, 4, 5, 6, 7; parent #1418, milestones #1419/#1420 reuse
 * this platform).
 *
 * Engineering + Operations carried through their real shipped documents at
 * 100/150/200% text, browser zoom and forced colours, on the device matrix
 * issue #1421 established — the same discipline
 * `text-scale-power-workflow.spec.js` (#1422) and
 * `helm-navigation-text-scale.spec.js` (#1423) used before this file. Power
 * allocation itself is NOT re-tested here — `text-scale-power-workflow.spec.js`
 * already carries the battleship's DEDICATED Power seat end to end and is the
 * tracer this issue reuses; the composite Engineering documents below still
 * mount the SAME `ph-power-controls`/`ph-battery-bar` pair, so this file's
 * reachability checks on them cover the composite surface's own layout
 * (a 3-column destroyer grid and a 2-column cruiser grid neither of which
 * #1422 laid out) without re-asserting the control's own contract twice.
 *
 * ── All supported family variants (issue #1425 acceptance criterion 1) ─────
 *
 * Inventory taken directly from each hull's `assets/entities/alliance_*.toml`
 * `[[system]]` table (never from HTML presence alone — a document can carry
 * markup for a system a hull does not author; it just stays `hidden`):
 *
 * | Hull       | repair | tractor | umbilical | dock | transporter | Console document(s) |
 * | ---------- | ------ | ------- | --------- | ---- | ----------- | -------------------- |
 * | Battleship | yes    | no      | no        | no   | no          | DEDICATED `repair.html` (+ DEDICATED `power.html`, covered by #1422) |
 * | Cruiser    | yes    | yes     | yes       | yes  | no          | COMPOSITE `engineering.html` (Power+Repair+Tractor+Umbilical); Dock lives on `helm.html` (Helm-owned) |
 * | Destroyer  | yes    | yes     | yes       | yes  | yes         | COMPOSITE `engineering.html` (Shields+Power+Repair+Tractor+Umbilical+Field-Repair-dispatch); Dock on `helm.html` |
 * | Courier    | yes    | no      | no        | no   | no          | no Engineering/Repair/Helm seat at all (`assets/entities/alliance_courier.toml`: captain, tactical only) — carries no changes, matching #1423's finding for the same hull |
 *
 * "Transport": the destroyer's `[transporter]` system (issue #1348, rescue
 * beam) has **no client console UI at all** — confirmed by an exhaustive grep
 * of `gui/` for `transporter`/`StartTransport`/`start_transport` returning
 * nothing; it is sim-only today (also recorded in the T3 engineering map).
 * The integrator brief's "Transport panels" is therefore read as the ACTUAL
 * transport-a-team-off-ship mechanism that does ship with a console: the
 * External repair-team dispatch capability (`[repair.external_dispatch]`,
 * issue #1161), rendered two ways depending on when a hull got it:
 *   - The LEGACY standalone dispatch/recall button+status+refusal
 *     (`gui/stations/repair-console.js` `ids.dispatchPanel`, and the
 *     destroyer's own tail in `gui/destroyer/engineering.console.js`) — shown
 *     on the battleship's dedicated Repair seat and the destroyer's composite
 *     Engineering seat.
 *   - The MODERN per-team dispatch-to-field-target card inside
 *     `ph-repair-teams` itself (issues #1384/#1386 — DISPATCH TO: the field
 *     target, and the "abroad" card once a team is there), which every hull
 *     with `[repair.external_dispatch]` gets automatically because
 *     `external_dispatch` is always forwarded into `ph-repair-teams`' own
 *     state regardless of whether a hull's document also mounts the legacy
 *     button (`gui/stations/engineering-console.js` `renderStation`).
 * `assets/entities/alliance_cruiser.toml` now authors `[repair.external_dispatch]`
 * (added by issue #1391, sharing the destroyer's range/rate — see the TOML's
 * own comment), so the cruiser DOES have field dispatch today, exclusively
 * through the modern per-team card: `gui/cruiser/engineering.console.js` and
 * `gui/cruiser/engineering.html` were never given the legacy standalone panel
 * (its own top comment still says "This hull authors no
 * `[repair.external_dispatch]`" — now stale as a design NOTE, though not as a
 * functional gap; flagged for the batch integrator rather than resolved here,
 * since restoring or deleting the legacy panel is a design choice outside a
 * text-scale issue). This file exercises BOTH mechanisms: the legacy panel on
 * the battleship and destroyer, and the modern per-team "abroad" card
 * (test 7) on a hull that only has that path.
 *
 * ── A real, fixed clipping bug (issue #1425 acceptance criterion 3) ────────
 *
 * `gui/cruiser/helm.html`'s `.dock-status` already carries
 * `flex: 1 1 auto; min-width: 0; overflow-wrap: anywhere;` — added when that
 * hull's Helm mounted a dock system (#1388) specifically because
 * `docked_to_name`/`available_target_name` are WORLD ENTITY NAME ids
 * (`src/core/messages.rs`: "coupled_target_name is a world entity name id"),
 * unbounded in length, sitting in a `display: flex` row next to a button. That
 * fix was never backported to the equivalent rows this issue also covers:
 * `gui/destroyer/helm.html`'s `.dock-status` AND `.tow-load-target`, and the
 * `.tractor-status` class shared by Tractor/Umbilical/(the destroyer's)
 * dispatch status on both `gui/destroyer/engineering.html` and
 * `gui/cruiser/engineering.html`, and `.dispatch-status` on
 * `gui/battleship/repair.html`. All five now carry the same treatment (this
 * issue's commit) — tests 1-4 below assert `scrollWidth <= clientWidth`
 * (never merely that horizontal document overflow stays near zero, which a
 * flex row silently escaping past its OWN box would not raise) using real
 * long world-entity-name strings.csv rows as target names, never fabricated
 * text.
 *
 * ── Which bundle ─────────────────────────────────────────────────────────
 * `/client/...`, i.e. `node scripts/build-client.mjs` output. Every document
 * here is pure HTML + `gui/*` modules with no WASM.
 */

// A NAMED subset of DEVICE_MATRIX, for the same reason #1422/#1423 use one:
// PRD #1418's own starting case (390x844 phone, which ALSO exercises the
// destroyer Engineering seat's phone-only REPAIR|OPS segment), the
// interim landscape tablet, and — issue #1425 acceptance criterion 4 calls
// out "650x450 and the 320 floor" by name — both split-pane-relevant rows.
const CARRIED_ON = [
  'phone-390x844-portrait',
  'tablet-650x450',
  'tablet-1280x720-interim-landscape',
  'native-split-pane-floor',
];

const device = (id) => {
  const found = DEVICE_MATRIX.find((d) => d.id === id);
  if (!found) throw new Error(`device-matrix.mjs has no row '${id}'`);
  return found;
};

/** See text-scale-power-workflow.spec.js for why a split-pane row scales its
 *  WIDTH floor with the text multiplier rather than being tested at 320x320. */
function viewportFor(entry, scale) {
  if (entry.kind !== 'split-pane') return { width: entry.width, height: entry.height };
  return { width: Math.round(entry.width * scale), height: entry.height };
}

/** Apply a text scale through the SHIPPED profile modules, not by poking CSS. */
async function applyTextScale(page, scale) {
  await page.evaluate(async (value) => {
    const { applyEffectsToRoot, resolveEffects } = await import(
      '/client/gui/accessibility-profile.js');
    applyEffectsToRoot(
      document.documentElement,
      resolveEffects({ presentation: { textScale: value } }),
    );
  }, scale);
}

// Real strings.csv world-entity-name rows (never fabricated text), chosen for
// length — the exact class of value `coupled_target_name`/`docked_to_name`/
// `available_target_name`/external_dispatch `target_name` actually carries.
const LONG_NAME_TRACTOR = 'world.falling_skyway.entity.corridor_obstruction.name'; // "[Corridor Obstruction]"
const LONG_NAME_DOCK = 'world.probe_fleet_comms_choice.entity.claimant_far.name'; // "[PROBE FAR CORRIDOR CLAIMANT]" (30 chars)
const LONG_NAME_DISPATCH = 'world.falling_skyway.entity.castaway_lifeboat.name'; // "[Castaway Lifeboat]"

// ── Payloads: real authored System ids per hull TOML, not generic fixtures
// (issue #1425 acceptance criterion 6: "tests retain authored System
// identity") — 'repair', 'tractor', 'umbilical', 'dock', 'power-reactor' are
// literally the `id = "..."` lines in assets/entities/alliance_*.toml.

const DESTROYER_ENGINEERING_PAYLOAD = {
  system_ids: ['shields-system', 'power-reactor', 'repair', 'tractor', 'umbilical'],
  system_families: {
    'shields-system': 'shields',
    'power-reactor': 'power',
    repair: 'repair',
    tractor: 'tractor',
    umbilical: 'umbilical',
  },
  systems: {
    'shields-system': { facings: [{ id: 'fore', pct: 0.8 }], focused_facing: 'fore', shields_auto: false, threat_bearing: 118 },
    'power-reactor': {
      consoles: [
        { id: 'helm', label: 'PROPULSION', level: 2, commanded_level: 2, min_level: 1, max_level: 4 },
        { id: 'weapons', label: 'WEAPONS', level: 1, commanded_level: 3, min_level: 0, max_level: 4 },
        { id: 'shields', label: 'SHIELDS', level: 0, commanded_level: 0, min_level: 0, max_level: 4 },
      ],
      power_auto: false, battery_online: true, charging: true, battery_charge: 58, battery_max: 100,
    },
    repair: {
      overall_hull: { pct: 0.64, destroyed_pct: 0.08 },
      core_systems: [{ id: 'core-1', label: 'Core' }],
      teams: [{ id: 0, status: 'idle' }, { id: 1, status: 'idle' }],
      repair_auto: false,
      dispatch_targets: [{ id: 'helm', label: 'Helm' }],
      damaged_systems: [],
      // team_idx 0 makes team 0 read ABROAD through ph-repair-teams' own
      // per-team card (test 7); target is non-null so the legacy dispatch
      // panel is simultaneously in its WORKING state (test 3).
      external_dispatch: { team_idx: 0, target: 'ext-1', target_name: LONG_NAME_DISPATCH, range: 400, refusal: null },
    },
    tractor: { engaged: true, coupled_target_name: LONG_NAME_TRACTOR, range: 500, refusal: null },
    umbilical: { running: true, rate: 12, operator_level: 3, partner_level: 2, refusal: null },
  },
};

const CRUISER_ENGINEERING_PAYLOAD = {
  system_ids: ['power-reactor', 'repair', 'tractor', 'umbilical'],
  system_families: {
    'power-reactor': 'power',
    repair: 'repair',
    tractor: 'tractor',
    umbilical: 'umbilical',
  },
  systems: {
    'power-reactor': {
      consoles: [
        { id: 'helm', label: 'PROPULSION', level: 3, commanded_level: 3, min_level: 1, max_level: 4 },
        { id: 'weapons', label: 'WEAPONS', level: 2, commanded_level: 2, min_level: 0, max_level: 4 },
      ],
      power_auto: false, battery_online: true, charging: false, battery_charge: 44, battery_max: 100,
    },
    repair: {
      overall_hull: { pct: 0.81, destroyed_pct: 0 },
      core_systems: [],
      teams: [{ id: 0, status: 'repairing', target: 'weapons' }],
      repair_auto: false,
      dispatch_targets: [{ id: 'weapons', label: 'Weapons' }],
      damaged_systems: [{ id: 'weapons', label: 'Weapons', pct: 0.4 }],
      external_dispatch: null,
    },
    tractor: { engaged: false, range: 500, refusal: null },
    umbilical: { running: false, rate: 0, operator_level: null, partner_level: null, refusal: 'console.umbilical.idle' },
  },
};

const BATTLESHIP_REPAIR_PAYLOAD = {
  overall_hull: { pct: 0.55, destroyed_pct: 0.12 },
  core_systems: [{ id: 'core-1', label: 'Core' }],
  teams: [{ id: 0, status: 'idle' }, { id: 1, status: 'travelling', target: 'helm' }],
  repair_auto: false,
  dispatch_targets: [{ id: 'helm', label: 'Helm' }],
  damaged_systems: [{ id: 'helm', label: 'Helm', pct: 0.3 }],
  external_dispatch: { target: 'ext-1', target_name: LONG_NAME_DISPATCH, range: 400, refusal: null },
};

/** A cruiser Helm payload: a berth held DOCKED with a long world-entity name,
 *  so the status line's own text — not merely its container — is exercised. */
const CRUISER_HELM_PAYLOAD = {
  blips: [],
  range: 500, x: 0, z: 0, ship_heading: 90, speed: 0,
  on_screen: false,
  engine_port_thrust: 0, engine_stbd_thrust: 0,
  hostile_arcs: [], hostile_arc_color: null,
  helm_auto: false, lateral_auto: false,
  impulse_charge_progress: 0, boost_enabled: true, boost_active: false, boost_battery: 100,
  dock: {
    system_id: 'dock', available: false, engaged: false, docked: true,
    docked_to_name: LONG_NAME_DOCK,
  },
  tow_load: null,
};

const DESTROYER_ENGINEERING_URL = '/client/gui/destroyer/engineering.html';
const CRUISER_ENGINEERING_URL = '/client/gui/cruiser/engineering.html';
const BATTLESHIP_REPAIR_URL = '/client/gui/battleship/repair.html';
const CRUISER_HELM_URL = '/client/gui/cruiser/helm.html';

async function openConsole(page, url, consoleName, payload) {
  await page.goto(url);
  await page.waitForFunction(() => typeof window.__updateConsole === 'function');
  await page.evaluate(
    ({ name, state }) => window.__updateConsole(name, JSON.stringify(state)),
    { name: consoleName, state: payload },
  );
}

/**
 * Record every outbound admitted command on the real
 * `BroadcastChannel('phoenix-console-state')` these documents post to when
 * there is no parent/WASM host (ADR-0001 §3 target 4, standalone
 * `page.goto`) — the actual `sendAction` seam every Operations button uses,
 * not a stub. See `helm-navigation-text-scale.spec.js`'s identical helper.
 */
async function installCommandCapture(page) {
  await page.addInitScript(() => {
    window.__commands = [];
    const bc = new BroadcastChannel('phoenix-console-state');
    bc.onmessage = (e) => {
      if (e.data && e.data.type === 'console_action') {
        try { window.__commands.push(JSON.parse(e.data.payload)); } catch (_) { /* ignore */ }
      }
    };
    window.__commandCapture = bc;
  });
}

async function documentOverflow(page) {
  return page.evaluate(() => {
    const doc = document.documentElement;
    return {
      horizontal: doc.scrollWidth - doc.clientWidth,
      verticalScrollAvailable: doc.scrollHeight >= doc.clientHeight,
    };
  });
}

// ── 1. Destroyer Engineering: the fullest composite surface ────────────────

for (const id of CARRIED_ON) {
  const entry = device(id);

  test(`Destroyer Engineering (Shields/Power/Repair/Tractor/Umbilical/Dispatch) is reachable, non-clipping and readable on ${id}`, async ({ page }) => {
    for (const scale of TEXT_SCALES) {
      const viewport = viewportFor(entry, scale);
      await page.setViewportSize(viewport);
      await openConsole(page, DESTROYER_ENGINEERING_URL, 'engineering', DESTROYER_ENGINEERING_PAYLOAD);
      await applyTextScale(page, scale);
      await page.evaluate(() => document.fonts.ready);
      const where = `${id} @ ${scale}x (${viewport.width}x${viewport.height})`;
      // CSS `orientation: portrait` matches on height >= width of the ACTUAL
      // viewport, not `entry`'s static label — the split-pane row's own width
      // scales with the text multiplier (see viewportFor above), so
      // `native-split-pane-floor` is a 320x320 SQUARE (portrait, by that
      // rule) at 100% and only becomes landscape once its scaled width
      // overtakes its fixed 320px height. Computed per iteration rather than
      // read off `entry.orientation`, which would silently skip the segment
      // click this hull's ONE portrait-only case (the REPAIR|OPS segment)
      // needs at exactly that boundary.
      const isPortrait = viewport.height >= viewport.width;

      // Portrait is the ONE layout with the REPAIR|OPS segment
      // (#engineering-seg) that hides one column at a time; every other
      // layout shows both simultaneously. Tractor/Umbilical live under OPS;
      // the Field Repair dispatch panel lives under REPAIR, alongside the
      // hull bar and the team roster — see engineering.html's markup (it
      // trails <ph-repair-teams> inside #engineering-panel-repair, not
      // #engineering-panel-ops).
      if (isPortrait) {
        await page.click('#engineering-seg-ops');
      }

      for (const sel of ['#tractor-panel', '#umbilical-panel']) {
        const box = await page.evaluate((s) => {
          const el = document.querySelector(s);
          const r = el.getBoundingClientRect();
          return { hidden: el.hidden, boxed: r.width > 0 && r.height > 0 };
        }, sel);
        expect(box.hidden, `${where}: ${sel} shown`).toBe(false);
        expect(box.boxed, `${where}: ${sel} laid out`).toBe(true);
      }
      // The status lines carry the long world-entity names: assert the FIX
      // directly — the element's own content fits ITS box, not merely that
      // the document as a whole has no horizontal scrollbar (a flex child
      // can escape its row without ever pushing the document that far).
      for (const sel of ['#tractor-status', '#umbilical-status']) {
        const fit = await page.evaluate((s) => {
          const el = document.querySelector(s);
          return { text: el.textContent, fits: el.scrollWidth <= Math.ceil(el.getBoundingClientRect().width) + 1 };
        }, sel);
        expect(fit.text.length, `${where}: ${sel} carries real status text`).toBeGreaterThan(0);
        expect(fit.fits, `${where}: ${sel} "${fit.text}" fits its own box`).toBe(true);
      }

      if (isPortrait) {
        await page.click('#engineering-seg-repair');
      }
      for (const sel of ['#shield-facings', '#power-controls', '#battery-bar', '#hull-integrity', '#repair-teams', '#dispatch-panel']) {
        const box = await page.evaluate((s) => {
          const el = document.querySelector(s);
          const r = el.getBoundingClientRect();
          return r.width > 0 && r.height > 0;
        }, sel);
        expect(box, `${where}: ${sel} laid out`).toBe(true);
      }
      const dispatchFit = await page.evaluate(() => {
        const el = document.getElementById('dispatch-status');
        return { text: el.textContent, fits: el.scrollWidth <= Math.ceil(el.getBoundingClientRect().width) + 1 };
      });
      expect(dispatchFit.text.length, `${where}: dispatch-status carries real status text`).toBeGreaterThan(0);
      expect(dispatchFit.fits, `${where}: dispatch-status "${dispatchFit.text}" fits its own box`).toBe(true);

      const overflow = await documentOverflow(page);
      expect(overflow.horizontal, `${where}: no horizontal overflow`).toBeLessThanOrEqual(1);
      expect(overflow.verticalScrollAvailable, `${where}: vertical scroll available`).toBe(true);
    }
  });
}

// ── 2. Cruiser Engineering: 2-column composite, no Shields, no legacy dispatch

for (const id of CARRIED_ON) {
  const entry = device(id);

  test(`Cruiser Engineering (Power/Repair/Tractor/Umbilical) is reachable and readable on ${id}`, async ({ page }) => {
    for (const scale of TEXT_SCALES) {
      const viewport = viewportFor(entry, scale);
      await page.setViewportSize(viewport);
      await openConsole(page, CRUISER_ENGINEERING_URL, 'engineering', CRUISER_ENGINEERING_PAYLOAD);
      await applyTextScale(page, scale);
      await page.evaluate(() => document.fonts.ready);
      const where = `${id} @ ${scale}x (${viewport.width}x${viewport.height})`;

      for (const sel of ['#power-controls', '#battery-bar', '#hull-integrity', '#repair-teams', '#tractor-panel', '#umbilical-panel']) {
        const box = await page.evaluate((s) => {
          const el = document.querySelector(s);
          const r = el.getBoundingClientRect();
          return { hidden: !!el.hidden, boxed: r.width > 0 && r.height > 0 };
        }, sel);
        expect(box.boxed, `${where}: ${sel} laid out`).toBe(true);
      }
      // Cruiser has no field dispatch capability authored, so its idle
      // umbilical carries a refusal instead of a long name — confirm the
      // refusal banner (text, non-colour) is shown and laid out.
      const refusal = await page.evaluate(() => {
        const el = document.getElementById('umbilical-refusal');
        return { hidden: el.hidden, boxed: el.getBoundingClientRect().width > 0 };
      });
      expect(refusal.hidden, `${where}: umbilical refusal shown`).toBe(false);
      expect(refusal.boxed, `${where}: umbilical refusal laid out`).toBe(true);
      // No dispatch panel exists on this hull's document at all (the legacy
      // button was never mounted here — see the file-top inventory).
      expect(await page.evaluate(() => !!document.getElementById('dispatch-panel')),
        `${where}: no legacy dispatch panel on this hull`).toBe(false);

      const overflow = await documentOverflow(page);
      expect(overflow.horizontal, `${where}: no horizontal overflow`).toBeLessThanOrEqual(1);
      expect(overflow.verticalScrollAvailable, `${where}: vertical scroll available`).toBe(true);
    }
  });
}

// ── 3. Battleship Repair: dedicated seat, legacy dispatch (Transport) panel ─

for (const id of CARRIED_ON) {
  const entry = device(id);

  test(`Battleship Repair (hull/core/teams/Field-Repair dispatch) is reachable, non-clipping and readable on ${id}`, async ({ page }) => {
    for (const scale of TEXT_SCALES) {
      const viewport = viewportFor(entry, scale);
      await page.setViewportSize(viewport);
      await openConsole(page, BATTLESHIP_REPAIR_URL, 'repair', BATTLESHIP_REPAIR_PAYLOAD);
      await applyTextScale(page, scale);
      await page.evaluate(() => document.fonts.ready);
      const where = `${id} @ ${scale}x (${viewport.width}x${viewport.height})`;

      for (const sel of ['#hull-integrity', '#core-damage', '#repair-teams', '#dispatch-panel']) {
        const box = await page.evaluate((s) => {
          const el = document.querySelector(s);
          const r = el.getBoundingClientRect();
          return { hidden: !!el.hidden, boxed: r.width > 0 && r.height > 0 };
        }, sel);
        expect(box.hidden, `${where}: ${sel} shown`).toBe(false);
        expect(box.boxed, `${where}: ${sel} laid out`).toBe(true);
      }
      const status = await page.evaluate(() => {
        const el = document.getElementById('dispatch-status');
        return { text: el.textContent, fits: el.scrollWidth <= Math.ceil(el.getBoundingClientRect().width) + 1 };
      });
      expect(status.text.length, `${where}: dispatch-status carries real text`).toBeGreaterThan(0);
      expect(status.fits, `${where}: dispatch-status "${status.text}" fits its own box`).toBe(true);

      const overflow = await documentOverflow(page);
      expect(overflow.horizontal, `${where}: no horizontal overflow`).toBeLessThanOrEqual(1);
      expect(overflow.verticalScrollAvailable, `${where}: vertical scroll available`).toBe(true);
    }
  });
}

// ── 4. Cruiser Helm Dock: not covered by #1423 (which used the destroyer) ──

for (const id of CARRIED_ON) {
  const entry = device(id);

  test(`Cruiser Helm's Dock panel is reachable, non-clipping and readable on ${id}`, async ({ page }) => {
    for (const scale of TEXT_SCALES) {
      const viewport = viewportFor(entry, scale);
      await page.setViewportSize(viewport);
      await openConsole(page, CRUISER_HELM_URL, 'helm', CRUISER_HELM_PAYLOAD);
      await applyTextScale(page, scale);
      await page.evaluate(() => document.fonts.ready);
      const where = `${id} @ ${scale}x (${viewport.width}x${viewport.height})`;

      const dock = await page.evaluate(() => {
        const panel = document.getElementById('dock-panel');
        const btn = document.getElementById('dock-btn');
        const status = document.getElementById('dock-status');
        return {
          hidden: panel.hidden,
          panelBoxed: panel.getBoundingClientRect().width > 0,
          btnBoxed: btn.getBoundingClientRect().width > 0,
          docked: btn.classList.contains('docked'),
          statusText: status.textContent,
          statusFits: status.scrollWidth <= Math.ceil(status.getBoundingClientRect().width) + 1,
        };
      });
      expect(dock.hidden, `${where}: dock panel shown`).toBe(false);
      expect(dock.panelBoxed, `${where}: dock panel laid out`).toBe(true);
      expect(dock.btnBoxed, `${where}: dock button laid out`).toBe(true);
      expect(dock.docked, `${where}: docked state reflected`).toBe(true);
      expect(dock.statusText.length, `${where}: dock-status carries the long name`).toBeGreaterThan(0);
      expect(dock.statusFits, `${where}: dock-status "${dock.statusText}" fits its own box`).toBe(true);

      const overflow = await documentOverflow(page);
      expect(overflow.horizontal, `${where}: no horizontal overflow`).toBeLessThanOrEqual(1);
      expect(overflow.verticalScrollAvailable, `${where}: vertical scroll available`).toBe(true);
    }
  });
}

// ── 5. Keyboard: the Tractor engage/release path at 200% ───────────────────

test('the Tractor toggle is operable from the keyboard at 200% text on the destroyer Engineering seat', async ({ page }) => {
  await installCommandCapture(page);
  const entry = device('phone-390x844-portrait');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await openConsole(page, DESTROYER_ENGINEERING_URL, 'engineering', DESTROYER_ENGINEERING_PAYLOAD);
  await applyTextScale(page, 2);
  await page.click('#engineering-seg-ops');

  // Tab order and the button's own affordance are native <button> semantics
  // here (no roving tabindex to verify) — Enter/Space activates it exactly as
  // a click would, through the same `activateEngineeringAction` seam every
  // Operations control in this file uses.
  await page.evaluate(() => document.getElementById('tractor-btn').focus());
  await expect(page.evaluate(() => document.activeElement.id)).resolves.toBe('tractor-btn');
  await page.keyboard.press('Enter');
  await expect.poll(() => page.evaluate(() => window.__commands.length)).toBeGreaterThan(0);
  const sent = await page.evaluate(() => window.__commands[0]);
  // The payload started engaged (see DESTROYER_ENGINEERING_PAYLOAD), so the
  // one button must have sent release, not engage — proving the click
  // handler read the FRESH `.engaged` class this render pass set, not a
  // stale one from before the payload arrived.
  expect(sent.action).toBe('release_tractor');
});

// ── 6. Non-colour: engaged/docked/working differ by TEXT, and forced colours

test('Tractor/Dock/Dispatch engaged states are distinguishable without colour, and their borders survive forced colours', async ({ page }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await openConsole(page, DESTROYER_ENGINEERING_URL, 'engineering', DESTROYER_ENGINEERING_PAYLOAD);

  // Idle vs engaged: pull the SAME button through both states and diff their
  // own text content — the actual non-colour cue already shipped here (the
  // verb and the status line both flip), unlike the pre-#1423 Set-Waypoint
  // button, which kept identical text and relied on colour alone.
  const idlePayload = {
    ...DESTROYER_ENGINEERING_PAYLOAD,
    systems: {
      ...DESTROYER_ENGINEERING_PAYLOAD.systems,
      tractor: { engaged: false, range: 500, refusal: null },
    },
  };
  // tablet-1280x720-interim-landscape is landscape, so #engineering-seg is
  // `display: none` and both Repair/Operations columns are already on
  // screen — no segment tab to select, unlike the phone-portrait tests above.
  await openConsole(page, DESTROYER_ENGINEERING_URL, 'engineering', idlePayload);
  const idleText = await page.evaluate(() => ({
    btn: document.getElementById('tractor-btn').textContent,
    status: document.getElementById('tractor-status').textContent,
  }));
  await openConsole(page, DESTROYER_ENGINEERING_URL, 'engineering', DESTROYER_ENGINEERING_PAYLOAD);
  const engagedText = await page.evaluate(() => ({
    btn: document.getElementById('tractor-btn').textContent,
    status: document.getElementById('tractor-status').textContent,
  }));
  expect(engagedText.btn, 'the button label itself changes, not only its colour').not.toBe(idleText.btn);
  expect(engagedText.status, 'the status line changes too').not.toBe(idleText.status);

  // Forced colours: the button's resting border (shared by every state) must
  // stay visible — inherited from #1422's tokens.css forced-colors block,
  // which points --edge at CanvasText. Verified here rather than assumed,
  // because this Operations-panel button family was never itself exercised
  // under forced colours before this issue.
  await page.emulateMedia({ forcedColors: 'active' });
  const forced = await page.evaluate(() => {
    const cs = getComputedStyle(document.getElementById('tractor-btn'));
    return { borderWidth: parseFloat(cs.borderTopWidth), borderColour: cs.borderTopColor };
  });
  expect(forced.borderWidth, 'a real border under forced colours').toBeGreaterThan(0);
  expect(forced.borderColour, 'not a transparent border').not.toBe('rgba(0, 0, 0, 0)');
});

// ── 7. Transport (the modern per-team card): the abroad state at 200% ──────

test('a repair team abroad on the field-dispatch target reads correctly and does not clip at 200%', async ({ page }) => {
  const entry = device('phone-390x844-portrait');
  await page.setViewportSize({ width: entry.width, height: entry.height });
  await openConsole(page, DESTROYER_ENGINEERING_URL, 'engineering', DESTROYER_ENGINEERING_PAYLOAD);
  await applyTextScale(page, 2);
  await page.click('#engineering-seg-repair');
  await page.evaluate(() => document.fonts.ready);

  // Team 0 is the one external_dispatch.team_idx names as abroad (see the
  // payload comment above) — open its card the way a player taps a summary.
  const card = await page.evaluate(() => {
    const shadow = document.getElementById('repair-teams').shadowRoot;
    const cardTop = shadow.querySelector('[data-team-id="0"] .card-top');
    cardTop.click();
    const badge = shadow.querySelector('[data-team-id="0"] .status-badge');
    const targetLabel = shadow.querySelector('[data-team-id="0"] .target-label');
    return {
      status: badge.textContent,
      target: targetLabel.textContent,
      // A block-level span (not a flex row) — should already wrap, unlike
      // the flex-row bug fixed elsewhere in this issue; asserted here so a
      // future regression back to a flex layout would be caught.
      fits: targetLabel.scrollWidth <= Math.ceil(targetLabel.getBoundingClientRect().width) + 1,
    };
  });
  expect(card.target.length, 'the field target name is shown').toBeGreaterThan(0);
  expect(card.fits, `the abroad target label "${card.target}" fits its own box at 200%`).toBe(true);
});

// ── 8. Browser zoom, tested separately from the Phoenix text setting ───────

test('browser zoom is usable alongside the Phoenix text setting on the destroyer Engineering seat', async ({ browser }) => {
  const entry = device('tablet-1280x720-interim-landscape');
  for (const zoom of BROWSER_ZOOMS) {
    const context = await browser.newContext({
      viewport: {
        width: Math.round(entry.width / zoom),
        height: Math.round(entry.height / zoom),
      },
      deviceScaleFactor: zoom,
    });
    const page = await context.newPage();
    try {
      await openConsole(page, DESTROYER_ENGINEERING_URL, 'engineering', DESTROYER_ENGINEERING_PAYLOAD);
      const where = `browser zoom ${zoom * 100}%`;
      const boxedBefore = await page.evaluate(() => {
        const el = document.getElementById('tractor-panel');
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
      });
      expect(boxedBefore, where).toBe(true);

      await applyTextScale(page, 2);
      const boxedAfter = await page.evaluate(() => {
        const el = document.getElementById('tractor-panel');
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
      });
      expect(boxedAfter, `${where} + 200%`).toBe(true);
      const overflow = await documentOverflow(page);
      expect(overflow.horizontal, `${where} + 200%: no horizontal overflow`).toBeLessThanOrEqual(1);
    } finally {
      await context.close();
    }
  }
});
