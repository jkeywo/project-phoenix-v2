// build-mod-pack-fixtures.mjs — regenerate the committed mod-pack test fixtures
// (issue #986).
//
//   node scripts/build-mod-pack-fixtures.mjs          # (re)write the .zip files
//   node scripts/build-mod-pack-fixtures.mjs --check  # CI: are they current?
//
// ── What this builds, and why it must be byte-reproducible ───────────────────
//
// Each directory under tests/fixtures/mod-packs/src/<name>/ is zipped into
// tests/fixtures/mod-packs/<name>.zip. Those .zip bytes are validated from BOTH
// languages against the ONE archive:
//   - Rust  : src/world/mod_pack.rs `include_bytes!`s each and runs it through
//             `validate_mod_pack`, asserting accept/reject + finding category;
//   - JS    : editor/tests/mod-pack-export.test.js reads the same bytes through
//             `readStoreZip` and asserts the manifest round-trips.
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
import { createStoreZip } from '../editor/mod-pack-export.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

export const FIXTURE_DIR = 'tests/fixtures/mod-packs';
export const SRC_DIR = `${FIXTURE_DIR}/src`;

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

/** The fixture names — one per source subdirectory, sorted. */
async function fixtureNames() {
  const abs = path.join(root, SRC_DIR);
  const dirents = await readdir(abs, { withFileTypes: true });
  return dirents
    .filter((d) => d.isDirectory())
    .map((d) => d.name)
    .sort();
}

async function main() {
  const check = process.argv.includes('--check');
  const names = await fixtureNames();
  if (names.length === 0) {
    console.error(`[build-mod-pack-fixtures] no source dirs under ${SRC_DIR}`);
    process.exit(1);
  }

  let drift = 0;
  for (const name of names) {
    const entries = await collectEntries(path.join(root, SRC_DIR, name));
    const bytes = Buffer.from(buildFixtureZip(entries));
    const outRel = `${FIXTURE_DIR}/${name}.zip`;
    const outAbs = path.join(root, outRel);

    if (check) {
      if (!existsSync(outAbs)) {
        console.error(`[build-mod-pack-fixtures] ${outRel} is missing`);
        drift += 1;
        continue;
      }
      if (!bytes.equals(readFileSync(outAbs))) {
        console.error(`[build-mod-pack-fixtures] ${outRel} has drifted from its source dir`);
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
    console.error(`[build-mod-pack-fixtures] ${names.length} fixture(s) up to date with their source dirs`);
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
