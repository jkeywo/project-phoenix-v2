/**
 * scripts/ship-cards.mjs — put the ship in the ship picker (PRD #1023 module
 * 4, user story 3: "I want the ship picker to show me the ship I am choosing,
 * so that the choice feels meaningful rather than abstract").
 *
 * ── Why a build step and not an <img src> ──────────────────────────────────
 *
 * The imagery already exists and is already committed. Every playable hull's
 * rig sidecar ends its LOD ladder with a captured billboard atlas —
 * `assets/models/<stem>_lod3.png`, an 8-view yaw ring produced by
 * scripts/capture-billboards.mjs driving the native `capture-billboard` tool.
 * That is the "existing capture pipeline" the PRD names, so nothing here
 * renders anything: this only makes what the pipeline already produced
 * REACHABLE from the picker. Two things were in the way.
 *
 *   1. The phone cannot see it. The client page is a deterministic file copy
 *      (scripts/build-client.mjs) of a named list of asset directories, and
 *      `assets/models` is not on it — for good reason, since it is ~40 MB of
 *      GLB. So the atlases have to be copied in, and only the atlases.
 *
 *   2. Nothing on the wire says which atlas. The picker knows a hull by its
 *      `template_path`; the atlas is found by reading that entity TOML's
 *      `[mesh] model`, taking the model's stem, and reading the sidecar's
 *      `[[lod]] billboard`. Entity stem and model stem are NOT reliably the
 *      same — `ship_civilian_hauler.toml` uses `dynasty_courier.glb` — so
 *      guessing the filename client-side would be wrong the first time a hull
 *      shared a model. Publishing the model path on the catalogue wire is a
 *      Rust change, and this pass is visual/UX only.
 *
 * Resolving it at BUILD time costs one small JSON and keeps a single source of
 * truth: the sidecar. Nothing derived is committed, so there is no second copy
 * to go stale — re-run the build and the card follows whatever the sidecar now
 * points at.
 *
 * ── Output ────────────────────────────────────────────────────────────────
 *
 *   <out>/assets/ship-cards/<entity-stem>.png   the atlas, copied verbatim
 *   <out>/assets/ship-cards/index.json          template_path → { image, views, tile }
 *
 * `tile` is which yaw view the card shows. Tile 0 is stern-on and tile 4 is
 * bow-on (the capture orbits `view * TAU / views`), which makes tile 3 the
 * front-three-quarter — the view a hull is drawn in when someone wants it to
 * look like a ship rather than a diagram.
 *
 * Usage:
 *   node scripts/ship-cards.mjs <outDir>     # writes <outDir>/assets/ship-cards/
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'smol-toml';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

/** The yaw view a card shows. See the module note. */
export const CARD_TILE = 3;

/** Read and parse a TOML file, or null when it is absent or unparseable. */
function readToml(file) {
  try {
    return parse(fs.readFileSync(file, 'utf8'));
  } catch {
    return null;
  }
}

/**
 * The billboard atlas an entity template resolves to, or null.
 *
 * entity TOML → `[mesh] model` → `<stem>.model.toml` → `[[lod]] billboard`.
 * Every hop is allowed to be missing: an entity with no mesh, a model with no
 * sidecar, and a ladder that never reaches a billboard level are all ordinary
 * (asteroids, station props, anything still being authored) and simply mean
 * "no card art for this one".
 *
 * @param {string} root repo root
 * @param {string} templatePath e.g. `assets/entities/alliance_cruiser.toml`
 * @returns {{ atlas: string, views: number }|null} atlas is repo-relative.
 */
export function billboardFor(root, templatePath) {
  const entity = readToml(path.join(root, templatePath));
  const model = entity && entity.mesh && entity.mesh.model;
  if (!model) return null;
  const stem = path.basename(model).replace(/\.glb$/i, '');
  const sidecar = readToml(path.join(root, 'assets', 'models', `${stem}.model.toml`));
  const ladder = (sidecar && sidecar.lod) || [];
  const level = ladder.find((l) => l && typeof l.billboard === 'string');
  if (!level) return null;
  if (!fs.existsSync(path.join(root, level.billboard))) return null;
  const views = (level.capture && Number(level.capture.yaw_views)) || 0;
  // A ladder that packed no view count is not a yaw ring this can index into.
  if (!Number.isInteger(views) || views < 2) return null;
  return { atlas: level.billboard, views };
}

/**
 * Every hull a world offers as a playable choice, as repo-relative
 * `template_path` strings.
 *
 * Scoped to `[[available_ships]]` deliberately: that list IS the picker's
 * contents, so it is exactly the set worth shipping art for. Cards for every
 * entity with a sidecar would be 3.7 MB of atlases, most of them asteroids and
 * enemy hulls no picker will ever draw; these four are 0.9 MB.
 *
 * A world that will not parse is skipped rather than fatal — the card is a
 * decoration, and failing a build over one is the wrong trade. A mod pack's
 * world arrives at runtime and was never here to scan, so its hulls simply get
 * no art; ph-ship-picker renders the card without it.
 *
 * @param {string} [root] repo root
 * @returns {Set<string>}
 */
export function playableHulls(root = ROOT) {
  const dir = path.join(root, 'assets', 'worlds');
  const out = new Set();
  if (!fs.existsSync(dir)) return out;
  for (const file of fs.readdirSync(dir).sort()) {
    if (!file.endsWith('.toml')) continue;
    const world = readToml(path.join(dir, file));
    for (const ship of (world && world.available_ships) || []) {
      if (ship && typeof ship.template_path === 'string') out.add(ship.template_path);
    }
  }
  return out;
}

/**
 * Every playable hull that has card art, as an index keyed by the
 * `template_path` the catalogue wire uses.
 *
 * @param {string} [root] repo root
 * @returns {Object<string, {image: string, views: number, tile: number}>}
 */
export function shipCardIndex(root = ROOT) {
  const index = {};
  for (const templatePath of [...playableHulls(root)].sort()) {
    const found = billboardFor(root, templatePath);
    if (!found) continue;
    const stem = path.basename(templatePath).replace(/\.toml$/, '');
    index[templatePath] = {
      image: `assets/ship-cards/${stem}.png`,
      views: found.views,
      // Clamped so a hull captured with fewer views than the hero index still
      // gets a real view rather than an out-of-range strip offset.
      tile: Math.min(CARD_TILE, found.views - 1),
    };
  }
  return index;
}

/**
 * Write the card atlases and their index under `<out>/assets/ship-cards/`.
 *
 * @param {string} out destination root (a dist directory)
 * @param {string} [root] repo root
 * @returns {number} how many hulls got card art
 */
export function emitShipCards(out, root = ROOT) {
  const index = shipCardIndex(root);
  const dest = path.join(out, 'assets', 'ship-cards');
  fs.mkdirSync(dest, { recursive: true });
  for (const [templatePath, entry] of Object.entries(index)) {
    const found = billboardFor(root, templatePath);
    fs.copyFileSync(path.join(root, found.atlas), path.join(out, entry.image));
  }
  fs.writeFileSync(path.join(dest, 'index.json'), `${JSON.stringify(index, null, 2)}\n`);
  return Object.keys(index).length;
}

// CLI: `node scripts/ship-cards.mjs <outDir>`. Used by build-client.mjs for the
// phone page and by a Trunk post_build hook for the host page.
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const out = process.argv[2];
  if (!out) {
    console.error('usage: node scripts/ship-cards.mjs <outDir>');
    process.exit(2);
  }
  const n = emitShipCards(path.resolve(out));
  console.log(`ship cards → ${path.join(out, 'assets', 'ship-cards')} (${n} hulls)`);
}
