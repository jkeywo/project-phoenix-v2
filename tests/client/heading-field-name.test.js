/**
 * tests/client/heading-field-name.test.js — one name for the ship's heading.
 *
 * The ship's compass heading in degrees had two names. Weapons, sensors and
 * navigation published `ship_heading`; helm published `heading`. The radar
 * components split the same way — ph-tactical-radar and ph-sensor-radar read
 * `ship_heading`, ph-helm-radar read `heading` — and the console documents
 * papered over the disagreement by writing BOTH keys into a single state
 * literal with the same value on each:
 *
 *     heading: s.heading || 0, speed: …, ship_heading: s.heading || 0,
 *
 * which is how a rename gets abandoned half-done and stays that way. The dead
 * key costs nothing until someone reads it, believes it, and wires a new
 * console to the name that happens to be missing from the payload they were
 * handed (PRD #1023's defect list).
 *
 * `ship_heading` won: three of the four payloads and two of the three
 * components already used it.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  buildHelmConsoleState,
  buildWeaponsConsoleState,
  buildSensorsConsoleState,
  buildNavigationConsoleState,
} from '../../gui/console-state.js';

const REPO_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const GUI = path.join(REPO_ROOT, 'gui');

const BUILDERS = {
  helm: buildHelmConsoleState,
  weapons: buildWeaponsConsoleState,
  sensors: buildSensorsConsoleState,
  navigation: buildNavigationConsoleState,
};

describe('every console payload names the heading the same way', () => {
  for (const [name, build] of Object.entries(BUILDERS)) {
    it(`${name} publishes ship_heading and not heading`, () => {
      const payload = JSON.parse(build({ shipYaw: Math.PI / 2 }));
      expect(payload).toHaveProperty('ship_heading');
      expect(payload).not.toHaveProperty('heading');
    });
  }

  it('helm still reports the same degrees under the agreed name', () => {
    const payload = JSON.parse(buildHelmConsoleState({ shipYaw: Math.PI / 2 }));
    expect(payload.ship_heading).toBeCloseTo(90, 3);
  });
});

/** Every console document (the per-hull HTML that feeds the radar components). */
function consoleDocuments() {
  const out = [];
  for (const hull of fs.readdirSync(GUI, { withFileTypes: true })) {
    if (!hull.isDirectory() || hull.name === 'borders' || hull.name === 'components') continue;
    const dir = path.join(GUI, hull.name);
    for (const file of fs.readdirSync(dir)) {
      if (file.endsWith('.html')) out.push(path.join(dir, file));
    }
  }
  return out;
}

describe('no console document writes the heading under two names', () => {
  const docs = consoleDocuments();

  it('finds the console documents to check', () => {
    expect(docs.length).toBeGreaterThan(10);
  });

  for (const doc of docs) {
    const rel = path.relative(REPO_ROOT, doc).replace(/\\/g, '/');
    it(`${rel} sets ship_heading only`, () => {
      const source = fs.readFileSync(doc, 'utf8');
      // A bare `heading:` key — not `ship_heading:`, `target_heading:` or a
      // CSS/class name containing the word.
      const bare = source.match(/(^|[^_\w])heading:\s/g) || [];
      expect(bare).toEqual([]);
    });
  }
});
