// build-mod-pack-fixtures.mjs — regenerate the committed mod-pack test fixtures
// (issue #986).
//
//   node scripts/build-mod-pack-fixtures.mjs          # (re)write the .zip files
//   node scripts/build-mod-pack-fixtures.mjs --check  # CI: are they current?
//
// ── What this builds, and why it must be byte-reproducible ───────────────────
//
// Each directory under tests/fixtures/mod-packs/src/<name>/ is zipped into
// tests/fixtures/mod-packs/<name>.zip. Two fixtures (issue #991) are NOT a plain
// zip of their source dir — a deliberately CRC-corrupted archive and one built
// by the real editor exporter — and are handled by the special builders below;
// they are byte-reproducible under `--check` all the same. Those .zip bytes are
// validated from BOTH languages against the ONE archive:
//   - Rust  : src/world/mod_pack.rs `include_bytes!`s each and runs it through
//             `validate_mod_pack`, asserting accept/reject + finding category;
//   - JS    : editor/tests/mod-pack-export.test.js reads the committed bytes
//             through `readStoreZip`, and tests/smoke/mod-pack.spec.js uploads
//             them to the real host page — so the two consumers cannot drift.
// A cross-language agreement test is only meaningful if both sides read the
// EXACT same bytes, so the fixtures are committed and this generator is the sole
// author of them — never a hand-edited binary.
//
// `--check` re-derives every archive in memory and byte-compares it to what is
// committed, exactly like `scripts/generate-lods.mjs --check`. It writes
// nothing, so CI can gate on "the committed .zip matches its source dir" without
// trusting the runner to produce identical binaries by luck.
//
// ── The determinism contract ─────────────────────────────────────────────────
//
// The ZIP writer is `createStoreZip` from editor/mod-pack-export.js — the SAME
// writer the exporter ships and `readStoreZip` (both languages) reads. It emits
// store-only entries with a FIXED modification time/date, NO extra fields, and a
// CRC over content, so its output is a pure function of the ordered
// { path, text } entries. This script makes that input deterministic too:
//   - files are collected in sorted path order (stable across filesystems);
//   - text is read as UTF-8 and CRLF is normalised to LF, so a Windows checkout
//     and a Linux CI runner hash identical content (belt-and-braces with the
//     `* text=auto` / `*.toml eol=lf` rules in .gitattributes).
// The .zip outputs are marked binary in .gitattributes so git never rewrites
// their line endings.

import { readFile, writeFile, readdir, stat } from 'node:fs/promises';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createStoreZip, exportModPack } from '../editor/mod-pack-export.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

export const FIXTURE_DIR = 'tests/fixtures/mod-packs';
export const SRC_DIR = `${FIXTURE_DIR}/src`;

// ── Special fixtures (issue #991) ─────────────────────────────────────────────
//
// Two fixtures are NOT a plain store-only zip of their source dir, so they are
// built by the custom steps below and EXCLUDED from the normal source-dir loop.
// `--check` re-derives both the same way, so they are byte-reproducible too:
//
//   * `corrupt-crc` — the store-only zip of its source dir, then ONE deterministic
//     byte flip that breaks the first entry's stored CRC. It cannot round-trip
//     through the store-writer (that is the point), so it is corrupted here in a
//     documented, reproducible step rather than committed as an opaque blob.
//   * `editor-round-trip` — produced by the REAL editor exporter
//     (`exportModPack`, issue #759/#989) rather than hand-authored, proving the
//     exact bytes the editor writes are accepted by the host validator (#760).
//     It has no source dir; its input is `EDITOR_ROUND_TRIP_INPUT` below.

/** Source dirs handled by a custom builder, so the normal loop skips them. */
const SPECIAL_FROM_SRC = new Set(['corrupt-crc']);

/** The editor-exporter input for the `editor-round-trip` fixture. A minimal
 *  MOD-mode export: a `[pack]` identity matching the shipped base content
 *  (`phoenix-base` epoch 1) plus one world and one scenario. `exportModPack`
 *  serialises + validates it, so the committed archive is genuinely
 *  editor-produced. Kept here (not in a source dir) because the exporter takes
 *  parsed objects, not files. */
const EDITOR_ROUND_TRIP_INPUT = {
  pack: {
    format: 1,
    id: 'editor-round-trip',
    version: '1.0.0',
    name: 'Editor Round Trip',
    author: 'Fixture Author',
    description: 'Produced by the editor exporter and re-validated on the host (issue #991).',
    requires: { content_id: 'phoenix-base', content_epoch: 1 },
  },
  files: [
    {
      path: 'assets/worlds/editor_arena.toml',
      parsed: { global: { title: 'Editor Arena' }, anchors: {} },
    },
  ],
  scenarios: [{ id: 'editor_arena', world: 'assets/worlds/editor_arena.toml', label: 'Editor Arena' }],
};

/** The store-only zip of `tests/fixtures/mod-packs/src/corrupt-crc/`, with the
 *  first local file header's CRC-32 field flipped so the archive fails its CRC
 *  check. Offset 14 is that field (sig 0..4, version 4..6, flags 6..8,
 *  method 8..10, mod time 10..12, mod date 12..14, **CRC 14..18**). Both
 *  store-zip readers (Rust `read_store_zip`, JS `readStoreZip`) recompute the
 *  CRC over the entry data and reject the mismatch → `invalid-archive`. */
async function buildCorruptCrcZip() {
  const entries = await collectEntries(path.join(root, SRC_DIR, 'corrupt-crc'));
  const bytes = Buffer.from(buildFixtureZip(entries));
  bytes[14] ^= 0xff;
  return bytes;
}

/** The `editor-round-trip` archive, straight from the editor exporter. */
function buildEditorRoundTripZip() {
  const result = exportModPack(EDITOR_ROUND_TRIP_INPUT);
  if (!result.ok) {
    throw new Error(
      `editor-round-trip export unexpectedly failed: ${(result.errors || []).join('; ')}`,
    );
  }
  return Buffer.from(result.zip);
}

/**
 * Collect every file under `dir` as `{ path, text }`, where `path` is the
 * forward-slash path relative to `dir` and `text` has LF line endings. Sorted by
 * path so the archive order is stable.
 */
export async function collectEntries(dir) {
  const out = [];
  async function walk(abs, rel) {
    const names = (await readdir(abs)).sort();
    for (const name of names) {
      const childAbs = path.join(abs, name);
      const childRel = rel ? `${rel}/${name}` : name;
      const st = await stat(childAbs);
      if (st.isDirectory()) {
        await walk(childAbs, childRel);
      } else {
        const text = (await readFile(childAbs, 'utf8')).replace(/\r\n/g, '\n');
        out.push({ path: childRel, text });
      }
    }
  }
  await walk(dir, '');
  out.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  return out;
}

/** Build the deterministic .zip bytes for one already-collected entry list. */
export function buildFixtureZip(entries) {
  return createStoreZip(entries);
}

/** The plain source-dir fixture names — one per subdirectory (special ones
 *  handled by a custom builder are excluded), sorted. */
async function fixtureNames() {
  const abs = path.join(root, SRC_DIR);
  const dirents = await readdir(abs, { withFileTypes: true });
  return dirents
    .filter((d) => d.isDirectory() && !SPECIAL_FROM_SRC.has(d.name))
    .map((d) => d.name)
    .sort();
}

/**
 * Every fixture to write/check, as `{ name, bytes }` — the plain source-dir
 * zips plus the special fixtures (issue #991) — sorted by name so the output
 * order is stable.
 */
async function fixtureBuilds() {
  const builds = [];
  for (const name of await fixtureNames()) {
    const entries = await collectEntries(path.join(root, SRC_DIR, name));
    builds.push({ name, bytes: Buffer.from(buildFixtureZip(entries)) });
  }
  builds.push({ name: 'corrupt-crc', bytes: await buildCorruptCrcZip() });
  builds.push({ name: 'editor-round-trip', bytes: buildEditorRoundTripZip() });
  builds.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  return builds;
}

async function main() {
  const check = process.argv.includes('--check');
  const builds = await fixtureBuilds();
  if (builds.length === 0) {
    console.error(`[build-mod-pack-fixtures] no source dirs under ${SRC_DIR}`);
    process.exit(1);
  }

  let drift = 0;
  for (const { name, bytes } of builds) {
    const outRel = `${FIXTURE_DIR}/${name}.zip`;
    const outAbs = path.join(root, outRel);

    if (check) {
      if (!existsSync(outAbs)) {
        console.error(`[build-mod-pack-fixtures] ${outRel} is missing`);
        drift += 1;
        continue;
      }
      if (!bytes.equals(readFileSync(outAbs))) {
        console.error(`[build-mod-pack-fixtures] ${outRel} has drifted from its source`);
        drift += 1;
      }
    } else {
      await writeFile(outAbs, bytes);
      console.error(`[build-mod-pack-fixtures] wrote ${outRel} (${bytes.length} bytes)`);
    }
  }

  if (check) {
    if (drift) {
      console.error(
        `\n[build-mod-pack-fixtures] ${drift} fixture(s) out of date — ` +
          'run `node scripts/build-mod-pack-fixtures.mjs` and commit the rebuilt .zip file(s).',
      );
      process.exit(1);
    }
    console.error(`[build-mod-pack-fixtures] ${builds.length} fixture(s) up to date with their sources`);
  }
}

// Guard the CLI entry so importing this module (from tests, or `node -e`) never
// touches the filesystem — same contract as scripts/generate-lods.mjs.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    console.error(`[build-mod-pack-fixtures] ${err.message}`);
    process.exit(1);
  });
}
