/**
 * tests/client/npc-hull-console-coverage.test.js — issue #925.
 *
 * NPC hulls (`ship_harrow_*`, `ship_requiem_courier`) author `[[station]]`
 * blocks a human can be admitted to. Before #925 those seats authored no
 * `console` path, so `resolveConsoleUrl` returned null, `planMounts` skipped the
 * seat, and a human saw a blank station. This test locks in two guarantees over
 * the REAL hull TOML:
 *
 *   AC6 — every seat on every NPC hull now resolves to a console URL, so
 *         `planMounts` mounts an iframe for it (the inverse of mount-plan.test's
 *         "skips stations with no console URL").
 *
 *   AC4 — every fine system a seat owns belongs to a console *family* the seat's
 *         chosen console actually renders (per gui/console-families.js). A seat
 *         whose owned family has no home in its console fails LOUDLY here rather
 *         than mounting a silently-blank panel.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse as parseToml } from 'smol-toml';

import { planMounts } from '../../gui/mount-plan.js';
import { resolveConsoleUrl } from '../../gui/console-resolver.js';
import { consoleForSystemId } from '../../gui/console-state.js';
import { familiesForConsole, shapeForConsole } from '../../gui/console-families.js';

const REPO_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

const NPC_HULLS = [
  'ship_harrow_cruiser',
  'ship_harrow_destroyer',
  'ship_harrow_warhawk',
  'ship_harrow_patrol',
  'ship_requiem_courier',
];

/**
 * Load a hull's TOML and derive the two shapes the client sees at runtime:
 *  - shipStations: { stations: [{ id, name, console }] } — what the host sends
 *    in Welcome and what resolveConsoleUrl / planMounts consume.
 *  - stationSystems: { [stationId]: fineSystemId[] } — each `[[system]]` block's
 *    `station` field decides which seat owns it (ownerless / auto-generated
 *    systems, e.g. `shield-arc-all`, have no `station` and belong to no seat).
 */
function loadHull(name) {
  const toml = parseToml(
    fs.readFileSync(path.join(REPO_ROOT, 'assets', 'entities', `${name}.toml`), 'utf-8'),
  );
  const stations = (toml.station || []).map(s => ({
    id: s.id,
    name: s.name,
    console: s.console,
  }));
  const stationSystems = {};
  for (const st of stations) stationSystems[st.id] = [];
  for (const sys of toml.system || []) {
    if (!sys.station) continue; // ownerless / ai_only synthesised system
    (stationSystems[sys.station] ||= []).push(sys.id);
  }
  return { stations, stationSystems };
}

describe('NPC-hull console coverage (issue #925)', () => {
  for (const hull of NPC_HULLS) {
    describe(hull, () => {
      const { stations, stationSystems } = loadHull(hull);

      it('has at least one station and every station owns systems', () => {
        expect(stations.length).toBeGreaterThan(0);
        for (const st of stations) {
          expect(stationSystems[st.id].length).toBeGreaterThan(0);
        }
      });

      it('AC6 — planMounts mounts an iframe for every seat', () => {
        const plan = planMounts({ stations });
        // Every authored seat resolves to a console URL and is in the plan.
        for (const st of stations) {
          expect(resolveConsoleUrl({ stations }, st.id)).toBeTruthy();
        }
        expect(plan.map(m => m.stationId).sort())
          .toEqual(stations.map(s => s.id).sort());
        for (const entry of plan) {
          expect(entry.url).toBeTruthy();
          expect(entry.iframeId).toBeTruthy();
        }
      });

      it("AC4 — each seat's owned system families are all rendered by its console", () => {
        for (const st of stations) {
          const covered = familiesForConsole(st.console);
          // An authored console must be known to the spec map, else it cannot be
          // checked — that is itself a coverage failure.
          expect(covered, `console ${st.console} on ${hull}/${st.id} is not in CONSOLE_SPECS`)
            .toBeTruthy();
          const ownedFamilies = new Set(
            stationSystems[st.id].map(consoleForSystemId),
          );
          for (const fam of ownedFamilies) {
            // A null family means a fine system id maps to no console family at
            // all — it can render nowhere; treat it as an uncovered family.
            expect(covered, `${hull}/${st.id} owns family ${fam} not covered by ${st.console}`)
              .toContain(fam);
          }
        }
      });

      it("AC4 — each seat's payload SHAPE matches its console (flat↔single-family, keyed↔multi-family)", () => {
        for (const st of stations) {
          // buildConsoleStateInner emits a FLAT payload for a single-family seat
          // and a system-id-KEYED payload for a multi-family seat. The console
          // must consume that exact shape or it renders blank — the defect the
          // first #925 pass shipped (single-family Harrow seats pointed at the
          // keyed destroyer consoles). Assert the two agree.
          const ownedFamilies = new Set(
            stationSystems[st.id].map(consoleForSystemId).filter(f => f !== null),
          );
          const expectedShape = ownedFamilies.size > 1 ? 'keyed' : 'flat';
          const consoleShape = shapeForConsole(st.console);
          expect(consoleShape, `console ${st.console} on ${hull}/${st.id} has no declared shape in CONSOLE_SPECS`)
            .toBeTruthy();
          expect(
            consoleShape,
            `${hull}/${st.id} owns ${ownedFamilies.size} console families → needs a ${expectedShape} payload console, but ${st.console} consumes ${consoleShape}`,
          ).toBe(expectedShape);
        }
      });
    });
  }
});
