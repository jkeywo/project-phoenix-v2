import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { parse } from 'smol-toml';
import { deriveTwoShipWorld, requirePreparedChoices, SOURCE_WORLD, OUTPUT_WORLD } from '../../scripts/prepare-gm-live-event.mjs';

const source = readFileSync(new URL('../../assets/worlds/combat_test.toml', import.meta.url), 'utf8');
const slot = readFileSync(new URL('../../docs/acceptance/fixtures/1320-second-ship.toml', import.meta.url), 'utf8');

describe('two-ship GM live event preparation', () => {
  it('retains the full real scenario and script while adding exactly one authored Fleet row', () => {
    const original = parse(source);
    const text = deriveTwoShipWorld(source, slot);
    const prepared = parse(text);
    const appended = prepared.entity.pop();
    expect(appended).toEqual(parse(slot).entity[0]);
    // Remove only the intentional self-reference rewrite if the shipped world
    // already contains the integrated M3 authoring when this test runs.
    for (const route of prepared.gm_comms_route ?? []) {
      for (const hail of route.hail ?? []) {
        if (hail.script_path?.startsWith(`${OUTPUT_WORLD}#`)) {
          hail.script_path = `${SOURCE_WORLD}${hail.script_path.slice(OUTPUT_WORLD.length)}`;
        }
      }
    }
    expect(prepared).toEqual(original);
    expect(parse(text).entity.filter(row => row.spawn_on === 'game_start')).toHaveLength(2);
    expect(parse(text).script.setup).toBe(original.script.setup);
    expect(text).toBe(deriveTwoShipWorld(source, slot));
    expect(source).toBe(readFileSync(new URL('../../assets/worlds/combat_test.toml', import.meta.url), 'utf8'));
  });

  it('rewrites the real authored hail binding to the generated setup without touching comments or script text', () => {
    const comment = `# Keep this commentary reference: ${SOURCE_WORLD}#script.setup\n`;
    const example = `${comment}${source}\n[[gm_comms_route]]\nid = "self-reference-test"\n[[gm_comms_route.hail]]\nid = "briefing"\nscript_path = "${SOURCE_WORLD}#script.setup" # keep trailing comment\n`;
    const result = deriveTwoShipWorld(example, slot);
    expect(result.startsWith(comment)).toBe(true);
    expect(result).toContain(`script_path = "${OUTPUT_WORLD}#script.setup" # keep trailing comment`);
    expect(parse(result).script.setup).toBe(parse(example).script.setup);
    expect(parse(result).gm_comms_route.at(-1).hail[0].script_path).toBe(`${OUTPUT_WORLD}#script.setup`);
  });

  it('refuses a second preparation pass or changed source topology', () => {
    expect(() => deriveTwoShipWorld(deriveTwoShipWorld(source, slot), slot)).toThrow('exactly one GameStart');
    expect(() => deriveTwoShipWorld(source.replace(/^spawn_on = "game_start"$/m, 'spawn_on = "immediate"'), slot))
      .toThrow('exactly one GameStart');
  });

  it('refuses a colliding identity and an unselectable second hull', () => {
    expect(() => deriveTwoShipWorld(source, slot.replace('gm-live-event-player-2', 'player-ship'))).toThrow('collides');
    expect(() => deriveTwoShipWorld(source, slot.replace('alliance_cruiser.toml', 'missing.toml'))).toThrow('hull choices');
  });

  it('does not call an incomplete directing palette ready for preparation', () => {
    const incomplete = parse(source);
    delete incomplete.gm_comms_route;
    expect(() => requirePreparedChoices(incomplete)).toThrow('Prepared Combat Test is missing');
  });
});
