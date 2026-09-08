// Prepare #1320's opt-in two-ship event from the integrated Combat Test.
// The source world and base catalogue remain unchanged. No runtime acceptance
// is implied by successful generation; see docs/acceptance/1320-gm-live-event.md.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parse } from 'smol-toml';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const SOURCE_WORLD = 'assets/worlds/combat_test.toml';
export const OUTPUT_WORLD = 'assets/worlds/prepared/gm_live_event_two_ship.toml';
export const OUTPUT_MANIFEST = 'assets/scenarios.gm-live-event.toml';
const SLOT = 'docs/acceptance/fixtures/1320-second-ship.toml';
const sha256 = text => createHash('sha256').update(text).digest('hex');
const requireThat = (condition, message) => { if (!condition) throw new Error(message); };

export function requirePreparedChoices(world) {
  const required = {
    gm_role_preset: ['directing', 'observer'],
    gm_palette: ['relief-cruiser'],
    gm_objective_palette: ['gm-relief-rendezvous', 'gm-relief-cover'],
    gm_npc_doctrine_palette: ['raider-regroup', 'raider-assault'],
    gm_comms_route: ['starbase-selected', 'starbase-fleet'],
  };
  for (const [table, ids] of Object.entries(required)) {
    for (const id of ids) {
      requireThat(world[table]?.some(row => row.id === id),
        `Prepared Combat Test is missing ${table} ${id}; integrate #1316/#1317 authoring first`);
    }
  }
  requireThat(world.gm_palette.find(row => row.id === 'relief-cruiser')
    .variant?.some(row => row.id === 'removable'), 'Missing removable relief-cruiser variant');
  requireThat(world.gm_comms_route.find(row => row.id === 'starbase-selected')
    .hail?.some(row => row.id === 'defence-briefing'
      && row.script_path === `${SOURCE_WORLD}#script.setup`),
  'Missing defence-briefing hail bound to the source setup script');
  const setup = world.script?.setup ?? '';
  for (let wave = 1; wave <= 8; wave++) {
    requireThat(setup.includes(`.gm_controls("release_wave_${wave}",`),
      `Missing prepared wave ${wave} control`);
  }
}

export function deriveTwoShipWorld(source, slotText) {
  const original = parse(source);
  const slotDocument = parse(slotText);
  requireThat(Object.keys(slotDocument).length === 1 && slotDocument.entity?.length === 1,
    'The second-ship fixture must contain exactly one entity table');
  const slot = slotDocument.entity[0];
  const initialSlots = original.entity?.filter(row => row.spawn_on === 'game_start') ?? [];
  requireThat(initialSlots.length === 1, 'Source must have exactly one GameStart row; review changed Fleet topology');
  requireThat(slot.spawn_on === 'game_start' && !slot.when && typeof slot.id === 'string',
    'The second ship must be an unconditional named GameStart row');
  requireThat(!original.entity.some(row => row.id === slot.id || row.name === slot.id),
    'The second ship identity collides with source content');
  requireThat(original.available_ships?.some(row => row.template_path === slot.template_path),
    'The second ship template must be one of the source world hull choices');
  requireThat(Array.isArray(slot.transform?.position) && slot.transform.position.length === 3
    && slot.transform.position.every(Number.isFinite), 'The second ship needs an authored finite position');

  const expected = structuredClone(original);
  expected.entity.push(slot);
  let rewritten = source;
  // Rewrite only the authored self-reference, not the embedded Rhai program
  // or a generic occurrence of the source path in comments/string literals.
  for (const route of expected.gm_comms_route ?? []) {
    for (const hail of route.hail ?? []) {
      if (hail.script_path?.startsWith(`${SOURCE_WORLD}#`)) {
        const oldPath = hail.script_path;
        hail.script_path = `${OUTPUT_WORLD}${oldPath.slice(SOURCE_WORLD.length)}`;
        const escaped = oldPath.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
        const assignment = new RegExp(`^([ \\t]*script_path[ \\t]*=[ \\t]*)(["'])${escaped}\\2`, 'gm');
        rewritten = rewritten.replace(assignment, (_, prefix, quote) => `${prefix}${quote}${hail.script_path}${quote}`);
      }
    }
  }
  const newline = source.includes('\r\n') ? '\r\n' : '\n';
  const result = `${rewritten}${newline}# Prepared two-ship event; source Combat Test is unchanged.${newline}${slotText.replace(/\r?\n/g, newline)}`;
  requireThat(isDeepStrictEqual(parse(result), expected),
    'Preparation changed content beyond the second ship and authored hail self-references');
  return result;
}

export async function prepareGmLiveEvent({ sourcePath = path.join(ROOT, SOURCE_WORLD), check = false } = {}) {
  const source = await readFile(sourcePath, 'utf8');
  const world = parse(source);
  requirePreparedChoices(world);
  const slotText = await readFile(path.join(ROOT, SLOT), 'utf8');
  const slot = parse(slotText).entity[0];
  for (const row of [world.entity.find(row => row.spawn_on === 'game_start'), slot]) {
    const hull = parse(await readFile(path.join(ROOT, row.template_path), 'utf8'));
    requireThat(hull.tags?.includes('ship'), `GameStart template is not a ship: ${row.template_path}`);
  }
  const generated = deriveTwoShipWorld(source, slotText);
  const content = parse(await readFile(path.join(ROOT, 'assets/scenarios.toml'), 'utf8')).content;
  requireThat(typeof content?.id === 'string' && Number.isInteger(content.epoch), 'Base catalogue content identity is missing');
  const manifest = `# Generated by scripts/prepare-gm-live-event.mjs; human acceptance is pending.\n[content]\nid = ${JSON.stringify(content.id)}\nepoch = ${content.epoch}\n\n[[scenario]]\nid = "gm_live_event_two_ship"\nworld = ${JSON.stringify(OUTPUT_WORLD)}\nships = [${JSON.stringify(slot.template_path)}]\n`;
  const artifacts = [[OUTPUT_WORLD, generated], [OUTPUT_MANIFEST, manifest]];
  for (const [relative, text] of artifacts) {
    const destination = path.join(ROOT, relative);
    if (check) {
      requireThat(await readFile(destination, 'utf8') === text, `Prepared asset is stale: ${relative}`);
    } else {
      await mkdir(path.dirname(destination), { recursive: true });
      await writeFile(destination, text, 'utf8');
    }
  }
  return {
    status: check ? 'prepared assets match' : 'prepared assets written',
    source: { path: sourcePath, sha256: sha256(source) },
    slot: { path: SLOT, sha256: sha256(slotText) },
    assets: artifacts.map(([assetPath, text]) => ({ path: assetPath, sha256: sha256(text) })),
    eventAcceptance: 'Pending: integrated runtime and human observations are required',
  };
}

async function main(args) {
  const options = {};
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--check') options.check = true;
    else if (args[i] === '--source' && args[i + 1]) options.sourcePath = path.resolve(args[++i]);
    else throw new Error('Usage: node scripts/prepare-gm-live-event.mjs [--check] [--source <prepared Combat Test>]');
  }
  console.log(JSON.stringify(await prepareGmLiveEvent(options), null, 2));
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exitCode = 1; });
}
