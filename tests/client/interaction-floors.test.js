/**
 * tests/client/interaction-floors.test.js — the interaction structural floors
 * (PRD #1168 T1 Interaction Foundations, issue #1175).
 *
 * A sibling family to control-floors.test.js. That file holds the floors PRD
 * #1023 named — TOUCH, TYPE, MOTION. This one holds the four #1168 adds, in
 * the same shape: enumerate the console surface, decide structurally what a
 * control is, and assert every one of them clears the floor or carries a
 * documented escape.
 *
 *   1. FOCUS-TOKEN ADOPTION. Every component adopts the shared control family,
 *      so its controls get #1170's focus ring. A component that hand-rolls its
 *      chrome instead has no visible focus — the regression #1170 ended.
 *   2. FOCUSABILITY. Every interactive control is a focusable thing — a native
 *      control, or a bare element the author gave a tabindex. A `pointerdown`
 *      on a plain <div> with neither is a control the keyboard cannot land on.
 *   3. KEYBOARD-REACHABILITY. No control is stranded — removed from the tab
 *      order with no roving to bring it back, or a surface with no focus target
 *      at all.
 *   4. ACCESSIBLE NAME + ROLE. Every control exposes a name (its text, a string
 *      id, or an aria-label); every custom composite exposes a role and a name.
 *
 * ── The allow-list, and why it is the point ────────────────────────────────
 *
 * #1170 converted ONE console — the Destroyer's Tactical (Weapons) — as the
 * tracer. The other consoles are debt. Rather than let that debt sit as
 * thirty-odd silently-failing assertions, it is written DOWN: DEBT below names
 * every component that fails a floor today, grouped by the family sweep that
 * will fix it (#1176/#1177/#1178). The floors run green against the list.
 *
 * The list is the mechanism the sweeps run against. "Done" for a sweep is
 * mechanical: convert the family's components, then delete the family's block
 * here. Two tests make that non-optional — a DEBT entry that STILL fails keeps
 * the floor green (its debt is acknowledged), but a DEBT entry that NO LONGER
 * fails breaks `the allow-list is honest`, so a sweep cannot quietly leave a
 * fixed component listed. The surface is enumerated live from the filesystem,
 * so a sweep adds nothing here to bring a new console under the floor — it only
 * ever removes.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import {
  evaluateSurface, focusFamilyAdoption, componentFiles, consoleDocuments, rel,
} from './interaction-scan.js';

// ── The tracer ────────────────────────────────────────────────────────────
// #1170's converted console. These MUST pass every floor with no allow-list
// entry — the whole point of a tracer is that it is already conformant.
const TRACER = [
  'gui/components/ph-phasers-controls.js',
  'gui/components/ph-blasters-controls.js',
  'gui/components/ph-torpedo-controls.js',
  'gui/components/ph-tactical-radar.js',
  'gui/destroyer/tactical.html',
];

/**
 * The debt, grouped by the sweep that clears it. A sweep deletes its whole
 * block when its family is converted (see the honesty test below).
 *
 * Each reason says what fails AND what the sweep does about it, so the entry
 * is a work item, not just a silence.
 */
const DEBT = {
  // #1176 (helm family and navigation composites) is CONVERTED — its block was
  // struck off here when the helm joysticks and navigation composites gained
  // focusable, named, keyboard-operable controls, proved by helm-nav-keyboard.
  // #1177 (combat family) is CONVERTED — its block was struck off here when
  // ph-power-controls and ph-shield-facings gained named, roled, keyboard-operable
  // controls, proved by combat-keyboard.test.js / combat-keyboard.spec.js.
  // #1178 (comms, sensors, ops and shared chrome) is CONVERTED — its block was
  // struck off here when ph-comms-hail-list, ph-objective-list and ph-ship-picker
  // became keyboard-navigable, named listboxes of native <button> option rows.
  // The family now passes every floor with no allow-list entry, proved by
  // tests/client/comms-ops-keyboard.test.js and tests/smoke/comms-ops-keyboard.spec.js.
};

/** DEBT flattened to `path -> reason`, the form the floors consult. */
const DEBT_BY_PATH = new Map(
  Object.values(DEBT).flatMap((block) => Object.entries(block)),
);

/**
 * Structural EXEMPTs — not debt, and no sweep will "fix" them, because there
 * is nothing to fix. Each is a real reason a floor cannot apply, keyed by path
 * like control-floors' EXEMPT map.
 */
const EXEMPT = new Map([
  ['gui/components/ph-radar.js',
    'the base scope canvas is never mounted directly — every use wraps it '
    + '(ph-tactical-radar / ph-helm-radar / ph-sensor-radar / ph-navigation-map), and the '
    + 'WRAPPER carries the group role, tabindex and name (see #1170\'s ph-tactical-radar). '
    + 'A role on ph-radar itself would be a second, wrong tab stop inside the group.'],
]);

/** Every surface under the floor: the components and the console documents. */
function surfaces() {
  return [
    ...componentFiles().map((file) => ({ file, isDocument: false })),
    ...consoleDocuments().map((file) => ({ file, isDocument: true })),
  ];
}

const isSkipped = (name) => DEBT_BY_PATH.has(name) || EXEMPT.has(name);

// ── 1. Focus-token adoption ─────────────────────────────────────────────────

describe('the focus-token adoption floor', () => {
  const components = componentFiles();

  it('finds the components to check', () => {
    expect(components.length).toBeGreaterThan(20);
  });

  for (const file of components) {
    const name = rel(file);
    const adoption = focusFamilyAdoption(fs.readFileSync(file, 'utf8'));
    if (!adoption.definesElement) continue;   // a helper module, not a component
    it(`${name} adopts the shared control family (so its controls get the focus ring)`, () => {
      // Directly via phAdoptConsoleStyles, or by extending a sibling that does.
      expect(adoption.ok).toBe(true);
    });
  }
});

// ── 2/3/4. Focusability, reachability, name+role ────────────────────────────

describe('the focusability floor', () => {
  for (const { file, isDocument } of surfaces()) {
    const name = rel(file);
    if (isSkipped(name)) continue;
    const result = evaluateSurface(fs.readFileSync(file, 'utf8'), { isDocument });
    if (!result.hasSurface) continue;
    it(`${name} exposes a focusable control for every interactive affordance`, () => {
      expect(result.floors.focusability).toEqual([]);
    });
  }
});

describe('the keyboard-reachability floor', () => {
  for (const { file, isDocument } of surfaces()) {
    const name = rel(file);
    if (isSkipped(name)) continue;
    const result = evaluateSurface(fs.readFileSync(file, 'utf8'), { isDocument });
    if (!result.hasSurface) continue;
    it(`${name} strands no control out of the tab order`, () => {
      expect(result.floors.reachability).toEqual([]);
    });
  }
});

describe('the accessible-name and role floor', () => {
  for (const { file, isDocument } of surfaces()) {
    const name = rel(file);
    if (isSkipped(name)) continue;
    const result = evaluateSurface(fs.readFileSync(file, 'utf8'), { isDocument });
    if (!result.hasSurface) continue;
    it(`${name} names every control and roles every composite`, () => {
      expect(result.floors.naming).toEqual([]);
    });
  }
});

// ── The tracer passes clean (AC #4) ─────────────────────────────────────────

describe('the tracer console passes with no allow-list entries', () => {
  for (const name of TRACER) {
    it(`${name} is conformant and unlisted`, () => {
      expect(DEBT_BY_PATH.has(name), `${name} must not be on the debt list`).toBe(false);
      expect(EXEMPT.has(name), `${name} must not be exempt`).toBe(false);
      const isDocument = name.endsWith('.html');
      const file = [...componentFiles(), ...consoleDocuments()].find((f) => rel(f) === name);
      expect(file, `${name} not found on the console surface`).toBeTruthy();
      const result = evaluateSurface(fs.readFileSync(file, 'utf8'), { isDocument });
      expect(result.floors).toEqual({ focusability: [], reachability: [], naming: [] });
    });
  }
});

// ── The allow-list is honest ────────────────────────────────────────────────

describe('the interaction allow-list is honest', () => {
  it('grants no debt entry without a reason', () => {
    for (const [path, reason] of DEBT_BY_PATH) {
      expect(reason.length, `${path} is listed with no reason`).toBeGreaterThan(30);
    }
  });

  it('grants no structural exemption without a reason', () => {
    for (const [path, reason] of EXEMPT) {
      expect(reason.length, `${path} is exempt with no reason`).toBeGreaterThan(30);
    }
  });

  it('every listed component exists on the console surface', () => {
    const known = new Set([...componentFiles(), ...consoleDocuments()].map(rel));
    for (const path of [...DEBT_BY_PATH.keys(), ...EXEMPT.keys()]) {
      expect(known.has(path), `${path} is listed but not on the surface`).toBe(true);
    }
  });

  // The forcing function for the sweeps: a DEBT entry that has stopped failing
  // must be REMOVED. If #1177 converts ph-power-controls but forgets to delete
  // its block, this fails with "no longer violates any floor" — which is the
  // exact reminder to strike it off.
  it('every debt entry still fails a floor (a fixed one must be struck off)', () => {
    const stale = [];
    for (const [path] of DEBT_BY_PATH) {
      const file = componentFiles().find((f) => rel(f) === path);
      if (!file) continue; // the existence test above reports a missing path
      const result = evaluateSurface(fs.readFileSync(file, 'utf8'), { isDocument: false });
      if (result.conformant) stale.push(path);
    }
    expect(stale, 'these no longer violate any floor — remove them from DEBT').toEqual([]);
  });
});
