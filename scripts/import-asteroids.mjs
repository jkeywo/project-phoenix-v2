// scripts/import-asteroids.mjs — turn raw asteroid geometry into shipped content.
//
//   node scripts/import-asteroids.mjs                   # rewrite the field configs only
//   node scripts/import-asteroids.mjs uncommon rare     # + import those rarity classes
//   node scripts/import-asteroids.mjs --plan            # print the work, write nothing
//   node scripts/import-asteroids.mjs rare --force      # overwrite files already on disk
//
// After importing a class, run `npm run lods` to materialise the `_lod1`/`_lod2`
// .glb files the sidecars this script writes declare, then `npm run lods:check`.
//
// ── Why this is a table and not a loop over 1..4 ────────────────────────────
//
// The predecessor hardcoded `for (let n = 1; n <= 4; n++)` over PPAsteroidCommon,
// one variant list, and one entity-TOML shape — so adding the uncommon and rare
// classes (issue #946) meant either forking it or hand-copying sixteen entity
// templates and forty-eight rig sidecars. Everything that varies between rocks
// now lives in the tables below, and adding a class or a size is an edit to
// data. Nothing here is a gameplay tunable in the AGENTS.md sense: every number
// it writes lands in a TOML a designer can then re-tune without re-running it.
//
// ── Never clobbers ─────────────────────────────────────────────────────────
//
// The shipped common sidecars have hand-tuned LOD ladders (a Blender voxel
// pre-pass, bespoke comments) that a regeneration would silently flatten, and
// their generated levels are hashed in scripts/lod-manifest.toml. So this
// script SKIPS any file that already exists and says so, unless --force. The
// only files it always rewrites are the two field configs' type arrays, which
// it composes from the class table in full.

import fs from "fs";
import path from "path";
import { execFileSync } from "child_process";
import { blenderCandidates } from "./generate-lods.mjs";

const root = process.cwd();

// ── The tables ─────────────────────────────────────────────────────────────

/**
 * Rarity classes. `weight` is the per-entry rarity weight written into the
 * field configs' `asteroid_type_paths`: weights are relative, so 0.1 means a
 * tenth as often as a common and 0.01 a hundredth (issue #946).
 *
 * `cosmetic` says whether the class also dresses the two backdrop layers.
 * Only the commons do: a cosmetic rock is non-targetable, hull-less scenery,
 * and an entry at 1:100 in a layer nobody shoots at is a model that ships and
 * is never seen. Rarity is a gameplay-legibility feature, so the rare and
 * uncommon classes are gameplay-only.
 *
 * `sizes` is the class's gameplay size ladder, in the order its entries are
 * written into the field configs. It is per class and not one global list for
 * the same reason `cosmetic` is: the `huge` size (issue #947) is a landmark
 * rock, and a landmark whose whole job is to be recognisable at range should be
 * the SAME four silhouettes every time. Scaling the uncommon and rare scans up
 * as well would ship eight more models that a player meets once an hour and
 * would make "that rock is enormous" and "that rock is unusual" the same
 * signal.
 *
 * `remeshVoxelSize` is the Blender voxel pre-pass the class's generated LOD
 * levels are made through (`scripts/blender-voxel-remesh.py`), or `null` for a
 * class that decimates cleanly without one. It lives here rather than in the
 * ladder because every size variant of one model shares one generated `.glb`,
 * and `scripts/generate-lods.mjs` refuses outright when two sidecars claim the
 * same output with different parameters. The commons need it; the uncommon and
 * rare scans arrived dense enough that the import's own collapse was enough.
 */
const CLASSES = [
  {
    name: "common",
    raw: "PPAsteroidCommon",
    count: 4,
    weight: 1.0,
    sizes: ["small", "large", "huge"],
    cosmetic: true,
    remeshVoxelSize: 0.03,
  },
  {
    name: "uncommon",
    raw: "PPAsteroidUncommon",
    count: 4,
    weight: 0.1,
    sizes: ["small", "large"],
    cosmetic: false,
    remeshVoxelSize: null,
  },
  {
    name: "rare",
    raw: "PPAsteroidRare",
    count: 4,
    weight: 0.01,
    sizes: ["small", "large"],
    cosmetic: false,
    remeshVoxelSize: null,
  },
];

/**
 * Size variants. `hull` absent ⇒ a cosmetic rock: no `[hull]`, no `[target]`.
 *
 * `rarity` multiplies the class weight for entries of this size, so a size can
 * be rarer than its class without touching the class's rarity story. `lodScale`
 * multiplies the ladder's switch distances: LOD bands are really an angular
 * threshold, so a rock three times the radius reaches the same on-screen size
 * three times further out, and a `huge` rock on the `large` bands would drop to
 * its far sphere while still filling a fifth of the viewscreen.
 */
const VARIANTS = {
  small: { label: "Small", radius: 2, hull: 30, colour: [0.55, 0.5, 0.42] },
  large: { label: "Large", radius: 4, hull: 100, colour: [0.6, 0.55, 0.45] },
  // Triple the large silhouette (issue #947), reusing the same four scans at a
  // larger authored scale rather than adding geometry.
  //
  // hull 300 is 3x large's, linear in radius rather than in volume. The rule is
  // "time to clear scales with how big the thing looks": a cruiser's two phaser
  // banks put out 8 hull/sec, so a large rock is ~12 s of sustained fire and a
  // huge one ~37 s — a commitment, but a clearable one. Cubing it to 2700 would
  // be 5.6 minutes on one rock and would read as indestructible scenery that
  // happens to have a health bar.
  //
  // rarity 0.1: at the class weight this size would be a third of every
  // gameplay rock in the field. At 0.1 it is ~4%, roughly one rock in twenty
  // three, which is what makes it read as a landmark instead of as the terrain.
  huge: {
    label: "Huge",
    radius: 12,
    hull: 300,
    colour: [0.65, 0.6, 0.48],
    rarity: 0.1,
    lodScale: 3,
  },
  cosmetic: { label: "Cosmetic", radius: 1, colour: [0.5, 0.45, 0.38] },
};

/** A size's rarity multiplier / LOD-distance multiplier, defaulting to 1. */
const sizeRarity = (variantKey) => VARIANTS[variantKey].rarity ?? 1;
const lodScale = (variantKey) => VARIANTS[variantKey].lodScale ?? 1;

/**
 * How a raw scan becomes the shipped near-LOD .glb.
 *
 * The raw library is not exported at a uniform density: the commons arrive at
 * ~31k triangles behind 2048px maps and ship after a texture-only pass, while
 * the uncommon and rare scans arrive at 500k-860k triangles behind the same
 * maps. `target_triangles` is therefore the commons' density, written down —
 * a rock that is already at or under it is passed through untouched.
 *
 * The decimation runs in Blender rather than through `gltf-transform
 * simplify`, and scripts/blender-decimate.py explains why at length: the
 * meshoptimizer path stalls at 116k triangles on these scans regardless of
 * its error bound, because gltf-transform's weld is bitwise and the exporter's
 * UV seams survive as borders it will not collapse across.
 */
const BASE_IMPORT = { targetTriangles: 31000, textureSize: 1024 };
const DECIMATE_SCRIPT = "scripts/blender-decimate.py";

/**
 * The distance-based LOD ladder (issue #914) every imported rock gets, matching
 * the profile the four commons share. `generate` levels are materialised by
 * `npm run lods` from the declarations this script writes into the sidecars —
 * this script never runs the decimation itself, so the manifest in
 * scripts/lod-manifest.toml only ever records files generate-lods.mjs made.
 *
 * These are the distances for a size whose `lodScale` is 1; a bigger size
 * multiplies them (see `VARIANTS`). The RATIOS are never scaled: the generated
 * `.glb` is shared by every size variant of the model, and two sidecars that
 * disagreed about how it is made is an error `generate-lods.mjs` refuses.
 */
const LADDER = {
  near_distance: 25.0,
  generated: [
    { suffix: "_lod1", max_distance: 150.0, ratio: 0.15, error: 0.01, texture_size: 512 },
    { suffix: "_lod2", max_distance: 300.0, ratio: 0.1, error: 0.1, texture_size: 256 },
  ],
  far: { shape: "sphere", scale: [1.0, 0.5, 1.0] },
};

const FIELD_CONFIGS = ["asteroid_field_main.toml", "asteroid_belt_axiom.toml"];

// ── CLI ────────────────────────────────────────────────────────────────────

const argv = process.argv.slice(2);
const FORCE = argv.includes("--force");
const PLAN = argv.includes("--plan");
const requested = argv.filter((a) => !a.startsWith("--"));
const unknown = requested.filter((r) => !CLASSES.some((c) => c.name === r));
if (unknown.length) {
  console.error(
    `[import-asteroids] unknown class(es): ${unknown.join(", ")} — known: ${CLASSES.map((c) => c.name).join(", ")}`,
  );
  process.exit(2);
}

const written = [];
const skipped = [];

function write(rel, contents) {
  const full = path.join(root, rel);
  if (fs.existsSync(full) && !FORCE) {
    skipped.push(rel);
    return;
  }
  if (PLAN) {
    written.push(`${rel} (plan)`);
    return;
  }
  fs.writeFileSync(full, contents);
  written.push(rel);
}

// ── glTF helpers ───────────────────────────────────────────────────────────

/** Read a .glb's position bounds straight out of its JSON chunk. */
function readGlbBounds(glbPath) {
  const buf = fs.readFileSync(glbPath);
  const jsonLen = buf.readUInt32LE(12);
  const json = JSON.parse(buf.toString("utf8", 20, 20 + jsonLen));
  const posAccessor = json.accessors.find((a) => a.type === "VEC3" && a.min && a.max);
  return { min: posAccessor.min, max: posAccessor.max };
}

function maxHorizontal({ min, max }) {
  return Math.max(Math.abs(min[0]), Math.abs(max[0]), Math.abs(min[2]), Math.abs(max[2]));
}

const CLI_ENTRY = "node_modules/@gltf-transform/cli/bin/cli.js";

/** Run the PINNED gltf-transform CLI — same resolution rule as generate-lods.mjs. */
function gltf(args) {
  const entry = path.join(root, CLI_ENTRY);
  if (!fs.existsSync(entry)) {
    throw new Error(`@gltf-transform/cli is not installed — run \`npm install\` first`);
  }
  execFileSync(process.execPath, [entry, ...args], { stdio: "inherit" });
}

/**
 * Locate Blender, sharing generate-lods.mjs's lookup order (PHOENIX_BLENDER,
 * then PATH, then the versioned Windows install dirs) so the two pre-passes
 * cannot disagree about which install they mean. Resolved lazily: a run that
 * only rewrites the field configs must not need Blender at all.
 */
let blenderPath = null;
function blender() {
  if (blenderPath) return blenderPath;
  let installedDirs = [];
  const foundation = path.win32.join(process.env.ProgramFiles || "C:\\Program Files", "Blender Foundation");
  if (process.platform === "win32" && fs.existsSync(foundation)) {
    installedDirs = fs.readdirSync(foundation);
  }
  for (const candidate of blenderCandidates({ env: process.env, installedDirs })) {
    try {
      execFileSync(candidate, ["--version"], { stdio: "ignore" });
      blenderPath = candidate;
      return candidate;
    } catch {
      /* try the next one */
    }
  }
  throw new Error(
    "no Blender found — set PHOENIX_BLENDER, put `blender` on PATH, or install it under " +
      '"C:\\Program Files\\Blender Foundation\\Blender <version>". Importing a raw scan needs it ' +
      `(see ${DECIMATE_SCRIPT}); the outputs are checked in, so nobody else does.`,
  );
}

/** raw scan → shipped near-LOD .glb: decimate in Blender, then cap texture size. */
function importBaseGlb(rawGlb, destRel) {
  const dest = path.join(root, destRel);
  if (fs.existsSync(dest) && !FORCE) {
    skipped.push(destRel);
    return;
  }
  if (PLAN) {
    written.push(
      `${destRel} (plan: decimate to ${BASE_IMPORT.targetTriangles} tris + resize ${BASE_IMPORT.textureSize})`,
    );
    return;
  }
  const tmp = path.join(root, "target", "asteroid-import");
  fs.mkdirSync(tmp, { recursive: true });
  const decimated = path.join(tmp, `${path.basename(destRel, ".glb")}.decimated.glb`);
  execFileSync(
    blender(),
    [
      "--background",
      "--factory-startup",
      "--python",
      DECIMATE_SCRIPT,
      "--",
      rawGlb,
      decimated,
      String(BASE_IMPORT.targetTriangles),
    ],
    { stdio: "inherit" },
  );
  gltf([
    "resize",
    decimated,
    dest,
    "--width",
    String(BASE_IMPORT.textureSize),
    "--height",
    String(BASE_IMPORT.textureSize),
  ]);
  fs.rmSync(decimated, { force: true });
  written.push(destRel);
}

// ── Emitters ───────────────────────────────────────────────────────────────

const num = (v) => (Number.isInteger(v) ? `${v}.0` : String(v));

/** `[base]`/`[extents]`/`[markers]` — the rig a variant places its mesh with. */
function rigBlock(scale, offsetY, bounds) {
  const { min, max } = bounds;
  const exMin = [min[0] * scale, min[1] * scale + offsetY, min[2] * scale];
  const exMax = [max[0] * scale, max[1] * scale + offsetY, max[2] * scale];
  const size = [exMax[0] - exMin[0], exMax[1] - exMin[1], exMax[2] - exMin[2]];
  return `[base]
offset = [ 0, ${offsetY}, 0 ]
rotation = [ 0, 0, 0 ]
scale = [ ${scale}, ${scale}, ${scale} ]

[extents]
min = [ ${exMin[0]}, ${exMin[1]}, ${exMin[2]} ]
max = [ ${exMax[0]}, ${exMax[1]}, ${exMax[2]} ]
size = [ ${size[0]}, ${size[1]}, ${size[2]} ]

[markers]
`;
}

/** The `[[lod]]` chain appended to a base sidecar. */
function ladderBlock(model, { remeshVoxelSize = null, scale = 1 } = {}) {
  const band = (d) => num(d * scale);
  const lines = [
    "",
    "# Distance-based LOD (issue #914): full GLB up close, two decimated steps, then",
    "# a shared procedural sphere far away. Levels omit `variant`, `radius` and",
    "# `colour` so each rock inherits them from the entity's own flat `[mesh]`.",
    "# The two decimated steps declare how they are regenerated (issue #919):",
    `#   node scripts/generate-lods.mjs ${model}`,
  ];
  if (scale === 1) {
    lines.push(
      "# Same bands and ratios as the four common asteroids, so every rock in a",
      "# field switches level at the same distance.",
    );
  } else {
    lines.push(
      `# The common asteroids' ratios, on bands ${scale}x further out (issue #947): a LOD`,
      "# switch is really an angular threshold, and this size is that much bigger,",
      "# so it reaches the same on-screen size that much further away. The ratios",
      "# themselves must NOT move — every size variant of this model shares one",
      "# generated .glb, and generate-lods.mjs refuses two sidecars that disagree",
      "# about how it is made.",
    );
  }
  if (remeshVoxelSize === null) {
    lines.push(
      "# No `remesh_voxel_size` here: the import already brought this mesh down to",
      "# the commons' density with a UV-preserving Blender collapse",
      "# (scripts/blender-decimate.py), which is what the voxel pre-pass was",
      "# standing in for on the stubborn commons.",
    );
  } else {
    lines.push(
      "# `remesh_voxel_size` is this class's Blender voxel pre-pass: a plain",
      "# regeneration at these ratios comes out LARGER than the source and the",
      "# growth gate refuses it. See scripts/blender-voxel-remesh.py, and run",
      "# `npm run lods -- --remesh` when regenerating.",
    );
  }
  lines.push(
    "[[lod]]",
    `max_distance = ${band(LADDER.near_distance)}`,
    `model = "assets/models/${model}.glb"`,
  );
  for (const level of LADDER.generated) {
    lines.push(
      "",
      "[[lod]]",
      `max_distance = ${band(level.max_distance)}`,
      `model = "assets/models/${model}${level.suffix}.glb"`,
      "",
      "[lod.generate]",
      `source = "assets/models/${model}.glb"`,
      `ratio = ${num(level.ratio)}`,
      `error = ${num(level.error)}`,
      `texture_size = ${level.texture_size}`,
    );
    if (remeshVoxelSize !== null) {
      lines.push(`remesh_voxel_size = ${remeshVoxelSize}`);
    }
  }
  lines.push(
    "",
    "[[lod]]",
    `shape = "${LADDER.far.shape}"`,
    `scale = [ ${LADDER.far.scale.map(num).join(", ")} ]`,
    "",
  );
  return lines.join("\n");
}

/** One gameplay or cosmetic entity template. */
function entityToml(entityName, modelName, variantKey) {
  const v = VARIANTS[variantKey];
  const gameplay = v.hull !== undefined;
  const parts = [
    `name = "entity.${entityName}.name"`,
    `tags = ["asteroid", "${gameplay ? "gameplay" : "cosmetic"}", "${variantKey}"]`,
    "",
  ];
  if (gameplay) {
    parts.push('[target]', 'tags = ["asteroid"]', 'threat_level = "none"', "");
  }
  parts.push("[collider]", 'shape = "Ball"', `radius = ${v.radius}`, "length = 0.0", "");
  if (gameplay) {
    parts.push("[hull]", `hull_integrity = ${v.hull}`, "");
  }
  parts.push(
    "[mesh]",
    "# The LOD ladder for this model lives in its rig sidecar (issue #914).",
    `model = "assets/models/${modelName}.glb"`,
    `variant = "${variantKey}"`,
    'shape = "sphere"',
    `radius = ${v.radius}`,
    `colour = [${v.colour.join(", ")}]`,
  );
  if (gameplay) {
    parts.push("", "[radar_appearance]", 'icon = "asteroid"');
  }
  return `${parts.join("\n")}\n`;
}

// ── Import one class ───────────────────────────────────────────────────────

/** Rows to add to assets/strings/strings.csv: `[id, context, en]`. */
const stringRows = [];

function importClass(cls) {
  for (let n = 1; n <= cls.count; n++) {
    const rawGlb = path.join(root, "raw", "models", `${cls.raw}${n}`, "base_basic_pbr.glb");
    if (!fs.existsSync(rawGlb)) {
      throw new Error(`missing raw geometry: ${rawGlb}`);
    }
    const modelName = `asteroid_${cls.name}_${n}`;
    importBaseGlb(rawGlb, `assets/models/${modelName}.glb`);

    const shipped = path.join(root, "assets", "models", `${modelName}.glb`);
    if (!fs.existsSync(shipped)) {
      // --plan, or the import was skipped and nothing is there to measure.
      continue;
    }
    const bounds = readGlbBounds(shipped);
    const horiz = maxHorizontal(bounds);

    const variants = [...cls.sizes, ...(cls.cosmetic ? ["cosmetic"] : [])];
    for (const variantKey of variants) {
      const v = VARIANTS[variantKey];
      const scale = v.radius / horiz;
      const offsetY = -(scale * bounds.max[1]) / 2;
      const rig = rigBlock(scale, offsetY, bounds);

      // The base sidecar carries the ladder; each generated level needs its
      // own sidecar too (the preloader fetches `<model>.<variant>.toml` for
      // every level it may load), and those carry the rig alone.
      const ladder = ladderBlock(modelName, {
        remeshVoxelSize: cls.remeshVoxelSize,
        scale: lodScale(variantKey),
      });
      write(`assets/models/${modelName}.${variantKey}.toml`, rig + ladder);
      for (const level of LADDER.generated) {
        write(`assets/models/${modelName}${level.suffix}.${variantKey}.toml`, rig);
      }

      const entityName = `${modelName}_${variantKey}`;
      write(`assets/entities/${entityName}.toml`, entityToml(entityName, modelName, variantKey));
      stringRows.push([
        `entity.${entityName}.name`,
        `assets/entities/${entityName}.toml → name (top level)`,
        `[${v.label} ${cls.name === "common" ? "" : `${cls.name[0].toUpperCase()}${cls.name.slice(1)} `}Asteroid ${n}]`,
      ]);
    }
  }
}

// ── strings.csv ────────────────────────────────────────────────────────────

/** Insert any missing rows, keeping the `entity.asteroid_*` block sorted by id. */
function updateStrings() {
  if (!stringRows.length || PLAN) return;
  const rel = "assets/strings/strings.csv";
  const file = path.join(root, rel);
  const lines = fs.readFileSync(file, "utf8").split("\n");
  const idOf = (line) => line.slice(0, line.indexOf(","));
  const have = new Set(lines.map(idOf));
  const missing = stringRows
    .filter(([id]) => !have.has(id))
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([id, context, en]) => `${id},${context},${en}`);
  if (!missing.length) return;

  let first = -1;
  let last = -1;
  lines.forEach((line, i) => {
    if (!idOf(line).startsWith("entity.asteroid_")) return;
    if (first < 0) first = i;
    last = i;
  });
  if (last < 0) throw new Error(`${rel}: no entity.asteroid_* rows to insert beside`);
  // Sorted INTO the block, not appended to the end of it: a new size lands
  // between `entity.asteroid_common_1_cosmetic` and `..._large`, and appending
  // would leave the block unsorted for every id that does not happen to sort
  // last. Merged in one pass so the rows keep their relative order.
  const block = lines.slice(first, last + 1);
  const merged = [];
  let m = 0;
  for (const line of block) {
    while (m < missing.length && idOf(missing[m]).localeCompare(idOf(line)) < 0) {
      merged.push(missing[m++]);
    }
    merged.push(line);
  }
  merged.push(...missing.slice(m));
  lines.splice(first, block.length, ...merged);
  fs.writeFileSync(file, lines.join("\n"));
  written.push(`${rel} (+${missing.length} rows)`);
}

// ── Field configs ──────────────────────────────────────────────────────────

/**
 * The sum of every gameplay entry's weight — the denominator the spawner's
 * weighted pick divides by, so "1 in N" in a generated comment is the number a
 * player would actually count.
 */
function totalGameplayWeight() {
  return CLASSES.reduce(
    (total, cls) =>
      total +
      cls.sizes.reduce((sub, s) => sub + cls.count * cls.weight * sizeRarity(s), 0),
    0,
  );
}

/**
 * Compose both type arrays from the class table and splice them into every
 * field config. Written in full every run: the arrays ARE the class table, so
 * a field config that disagreed with it would be the bug this replaces.
 */
function fieldArrays() {
  const gameplay = ["asteroid_type_paths = ["];
  for (const cls of CLASSES) {
    // Sizes at the class's own weight first, model-major, then any size that
    // carries its own rarity multiplier as a labelled group of its own. Two
    // groups rather than one interleaved list because they answer different
    // questions — "how rare is this rock's material" and "how rare is a rock
    // this big" — and a reader retuning one should not have to pick its lines
    // out of the other.
    const baseline = cls.sizes.filter((s) => sizeRarity(s) === 1);
    const scaled = cls.sizes.filter((s) => sizeRarity(s) !== 1);
    gameplay.push(
      `    # ${cls.name} — rarity weight ${num(cls.weight)}` +
        (cls.weight === 1 ? " (the baseline the others are relative to)" : ` ≈ 1:${Math.round(1 / cls.weight)} against a common`),
    );
    for (let n = 1; n <= cls.count; n++) {
      for (const variantKey of baseline) {
        gameplay.push(
          `    { path = "assets/entities/asteroid_${cls.name}_${n}_${variantKey}.toml", weight = ${num(cls.weight)} },`,
        );
      }
    }
    for (const variantKey of scaled) {
      const weight = cls.weight * sizeRarity(variantKey);
      gameplay.push(
        `    # ${cls.name}, ${variantKey} — ${sizeRarity(variantKey)}x the class weight (issue #947).`,
        `    # A rock this big is a landmark, not terrain: about 1 gameplay rock`,
        `    # in ${Math.round(totalGameplayWeight() / (cls.count * weight))} is one. At the class weight it would be nearly a third of them.`,
      );
      for (let n = 1; n <= cls.count; n++) {
        gameplay.push(
          `    { path = "assets/entities/asteroid_${cls.name}_${n}_${variantKey}.toml", weight = ${num(weight)} },`,
        );
      }
    }
  }
  gameplay.push("]");

  // The backdrop layers have no rarity tiers, so their entries stay in the
  // bare-string spelling — which is exactly `weight = 1.0` and keeps the
  // pre-#946 schema exercised by shipped content.
  const cosmetic = ["cosmetic_type_paths = ["];
  for (const cls of CLASSES.filter((c) => c.cosmetic)) {
    for (let n = 1; n <= cls.count; n++) {
      cosmetic.push(`    "assets/entities/asteroid_${cls.name}_${n}_cosmetic.toml",`);
    }
  }
  cosmetic.push("]");

  return { gameplay: gameplay.join("\n"), cosmetic: cosmetic.join("\n") };
}

function updateFieldConfigs() {
  const { gameplay, cosmetic } = fieldArrays();
  for (const file of FIELD_CONFIGS) {
    const rel = `assets/entities/${file}`;
    const full = path.join(root, rel);
    const before = fs.readFileSync(full, "utf8");
    const after = before
      .replace(/asteroid_type_paths = \[[^\]]*\]/, gameplay)
      .replace(/cosmetic_type_paths = \[[^\]]*\]/, cosmetic);
    if (after === before) continue;
    if (PLAN) {
      written.push(`${rel} (plan)`);
      continue;
    }
    fs.writeFileSync(full, after);
    written.push(rel);
  }
}

// ── Run ────────────────────────────────────────────────────────────────────

for (const cls of CLASSES.filter((c) => requested.includes(c.name))) {
  importClass(cls);
}
updateStrings();
updateFieldConfigs();

if (skipped.length) {
  console.error(`[import-asteroids] left ${skipped.length} existing file(s) alone (pass --force to overwrite):`);
  for (const s of skipped) console.error(`  ${s}`);
}
console.error(`[import-asteroids] wrote ${written.length} file(s):`);
for (const w of written) console.error(`  ${w}`);
if (requested.length) {
  console.error("\n  Next: `npm run lods` to build the declared _lod1/_lod2 levels, then `npm run lods:check`.");
}
