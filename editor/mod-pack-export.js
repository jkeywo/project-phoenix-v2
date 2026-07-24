/**
 * mod-pack-export.js — Validated TOML mod-pack exporter (issue #759).
 *
 * Builds a TOML-only, store-only (uncompressed) ZIP from a set of selected
 * authored files plus the required `[[scenario]]` manifest, and refuses the
 * export when the selected composition contains any definite authoring error.
 *
 * The pieces, all pure and unit-testable (no DOM, no file IO):
 *
 *   isAllowedContentPath(path)     — the export whitelist (AC1).
 *   validateManifestEntries(...)   — JS twin of `validate_manifest`
 *                                    (src/world/manifest.rs), resolving each
 *                                    root world within the pack (AC2).
 *   buildManifestToml(scenarios)   — serialise the `[[scenario]]` manifest.
 *   createStoreZip(entries)        — hand-rolled store-only ZIP writer.
 *   readStoreZip(bytes)            — its inverse, for round-trip checks and the
 *                                    upload surface (#760) to parse (AC4).
 *   exportModPack({...})           — the export chokepoint: mirrors the
 *                                    `SaveFlow._saveOne` admission gate
 *                                    (issue #757) but across ALL selected
 *                                    files (AC3), then packs the archive.
 *
 * The admission split comes from `partitionFindings` in validation.js — the
 * same primitive the per-file save gate uses — so a file that would be refused
 * a save can never slip through the exporter instead.
 */

import { stringify as tomlStringify, parse as tomlParse } from 'smol-toml';
import { validateFile, partitionFindings } from './validation.js';

/** The manifest path a mod pack always carries. */
export const MANIFEST_PATH = 'scenarios.toml';

/**
 * Whitelist of authored TOML paths a mod pack may carry. Structural (not
 * gameplay) — it names *where* content lives, never a tunable value. A file
 * whose path is outside this set must never reach the archive.
 *
 * Allowed:
 *   - assets/worlds/*.toml       selectable + supporting worlds
 *   - assets/entities/*.toml     entity templates
 *   - assets/factions/*.toml     faction definitions
 *   - assets/models/*.toml       model-rig sidecars (`<stem>.<variant>.toml`)
 *   - scenarios.toml             the required top-level manifest
 */
const ALLOWED_DIR_PREFIXES = [
  'assets/worlds/',
  'assets/entities/',
  'assets/factions/',
  'assets/models/',
];

/** Whether `path` is a world path allowed to be a manifest root world. */
export function isWorldContentPath(path) {
  return (
    typeof path === 'string' &&
    path.startsWith('assets/worlds/') &&
    path.endsWith('.toml') &&
    path.slice('assets/worlds/'.length).length > '.toml'.length &&
    !path.includes('..')
  );
}

/**
 * Whether `path` is a content path the exporter is allowed to include. The
 * top-level manifest (`scenarios.toml`) is allowed on its own; every other
 * file must sit directly under one of the supported `assets/*` directories,
 * end in `.toml`, and carry a non-empty file name with no path traversal.
 */
export function isAllowedContentPath(path) {
  if (typeof path !== 'string' || path.length === 0) return false;
  if (path.includes('..') || path.includes('\\')) return false;
  if (path === MANIFEST_PATH) return true;
  if (!path.endsWith('.toml')) return false;
  for (const prefix of ALLOWED_DIR_PREFIXES) {
    if (!path.startsWith(prefix)) continue;
    const name = path.slice(prefix.length);
    // Directly under the prefix (no further nesting) and a real file name.
    if (name.length > '.toml'.length && !name.includes('/')) return true;
    return false;
  }
  return false;
}

// ── CRC-32 (IEEE) ───────────────────────────────────────────────────────────

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c >>> 0;
  }
  return table;
})();

/** CRC-32 of a byte array, as an unsigned 32-bit integer. */
export function crc32(bytes) {
  let crc = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) {
    crc = CRC_TABLE[(crc ^ bytes[i]) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();

// ── Store-only ZIP writer ─────────────────────────────────────────────────────

/**
 * Build a store-only (compression method 0) ZIP archive.
 *
 * @param {Array<{ path: string, text: string }>} entries  File entries.
 * @returns {Uint8Array} the archive bytes.
 *
 * Deliberately minimal: no compression, no data descriptors, no ZIP64. Every
 * offset and length is known up front, so each local header carries its own
 * CRC and sizes. This keeps the writer small enough to audit and its output
 * trivially parseable by `readStoreZip` (and any conformant unzip).
 */
export function createStoreZip(entries) {
  const files = entries.map((e) => {
    const nameBytes = encoder.encode(e.path);
    const dataBytes = encoder.encode(e.text);
    return { nameBytes, dataBytes, crc: crc32(dataBytes) };
  });

  const localParts = [];
  const centralParts = [];
  let offset = 0;

  for (const f of files) {
    const localHeader = new Uint8Array(30 + f.nameBytes.length);
    const lv = new DataView(localHeader.buffer);
    lv.setUint32(0, 0x04034b50, true); // local file header signature
    lv.setUint16(4, 20, true); // version needed
    lv.setUint16(6, 0, true); // flags
    lv.setUint16(8, 0, true); // method: store
    lv.setUint16(10, 0, true); // mod time
    lv.setUint16(12, 0x21, true); // mod date (1980-01-01)
    lv.setUint32(14, f.crc, true);
    lv.setUint32(18, f.dataBytes.length, true); // compressed size
    lv.setUint32(22, f.dataBytes.length, true); // uncompressed size
    lv.setUint16(26, f.nameBytes.length, true);
    lv.setUint16(28, 0, true); // extra field length
    localHeader.set(f.nameBytes, 30);

    localParts.push(localHeader, f.dataBytes);

    const centralHeader = new Uint8Array(46 + f.nameBytes.length);
    const cv = new DataView(centralHeader.buffer);
    cv.setUint32(0, 0x02014b50, true); // central dir header signature
    cv.setUint16(4, 20, true); // version made by
    cv.setUint16(6, 20, true); // version needed
    cv.setUint16(8, 0, true); // flags
    cv.setUint16(10, 0, true); // method: store
    cv.setUint16(12, 0, true); // mod time
    cv.setUint16(14, 0x21, true); // mod date
    cv.setUint32(16, f.crc, true);
    cv.setUint32(20, f.dataBytes.length, true);
    cv.setUint32(24, f.dataBytes.length, true);
    cv.setUint16(28, f.nameBytes.length, true);
    cv.setUint16(30, 0, true); // extra length
    cv.setUint16(32, 0, true); // comment length
    cv.setUint16(34, 0, true); // disk number start
    cv.setUint16(36, 0, true); // internal attrs
    cv.setUint32(38, 0, true); // external attrs
    cv.setUint32(42, offset, true); // local header offset
    centralHeader.set(f.nameBytes, 46);
    centralParts.push(centralHeader);

    offset += localHeader.length + f.dataBytes.length;
  }

  const centralSize = centralParts.reduce((n, p) => n + p.length, 0);
  const centralOffset = offset;

  const eocd = new Uint8Array(22);
  const ev = new DataView(eocd.buffer);
  ev.setUint32(0, 0x06054b50, true); // EOCD signature
  ev.setUint16(4, 0, true); // disk number
  ev.setUint16(6, 0, true); // disk with central dir
  ev.setUint16(8, files.length, true); // entries on this disk
  ev.setUint16(10, files.length, true); // total entries
  ev.setUint32(12, centralSize, true);
  ev.setUint32(16, centralOffset, true);
  ev.setUint16(20, 0, true); // comment length

  const total =
    localParts.reduce((n, p) => n + p.length, 0) + centralSize + eocd.length;
  const out = new Uint8Array(total);
  let pos = 0;
  for (const part of localParts) {
    out.set(part, pos);
    pos += part.length;
  }
  for (const part of centralParts) {
    out.set(part, pos);
    pos += part.length;
  }
  out.set(eocd, pos);
  return out;
}

/**
 * Read a store-only ZIP produced by {@link createStoreZip} back into a map of
 * `{ path: text }`. Verifies each entry's stored CRC and rejects any entry
 * that is not compression method 0. Throws on a malformed archive.
 */
export function readStoreZip(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const files = {};
  let pos = 0;
  while (pos + 4 <= bytes.length && view.getUint32(pos, true) === 0x04034b50) {
    const method = view.getUint16(pos + 8, true);
    const crc = view.getUint32(pos + 14, true);
    const compSize = view.getUint32(pos + 18, true);
    const nameLen = view.getUint16(pos + 26, true);
    const extraLen = view.getUint16(pos + 28, true);
    const nameStart = pos + 30;
    const dataStart = nameStart + nameLen + extraLen;
    if (method !== 0) {
      throw new Error(`unsupported compression method ${method}`);
    }
    const name = decoder.decode(bytes.subarray(nameStart, nameStart + nameLen));
    const data = bytes.subarray(dataStart, dataStart + compSize);
    if (crc32(data) !== crc) {
      throw new Error(`CRC mismatch for "${name}"`);
    }
    files[name] = decoder.decode(data);
    pos = dataStart + compSize;
  }
  return files;
}

// ── Manifest ──────────────────────────────────────────────────────────────────

/**
 * Serialise `[[scenario]]` manifest entries to TOML. Emits `id`, `world`, and
 * an optional `label`, matching the schema `parse_manifest` reads
 * (`src/world/manifest.rs`).
 */
export function buildManifestToml(scenarios) {
  const scenario = [];
  for (const s of scenarios || []) {
    const entry = { id: String(s.id ?? ''), world: String(s.world ?? '') };
    if (typeof s.label === 'string' && s.label.length > 0) entry.label = s.label;
    scenario.push(entry);
  }
  return tomlStringify({ scenario });
}

/**
 * JS twin of `validate_manifest` (src/world/manifest.rs). Validates the
 * manifest's root-world entries against the SELECTED content: each entry's
 * world must resolve within the pack (be one of the exported files) and parse.
 *
 * @param {Array<{id, world, label?}>} scenarios  Manifest entries.
 * @param {Object<string,string>} contentByPath   path -> TOML text for every
 *   file that will be in the pack (used as the `resolve_world` closure).
 * @returns {Array<{ path, severity, message, category }>} findings.
 */
export function validateManifestEntries(scenarios, contentByPath) {
  const findings = [];
  const list = Array.isArray(scenarios) ? scenarios : [];

  if (list.length === 0) {
    findings.push({
      path: MANIFEST_PATH,
      severity: 'error',
      category: 'empty-manifest',
      message: 'scenario manifest declares no [[scenario]] entries',
    });
    return findings;
  }

  const seenIds = new Set();
  for (const entry of list) {
    const id = typeof entry.id === 'string' ? entry.id.trim() : '';
    const world = typeof entry.world === 'string' ? entry.world.trim() : '';

    if (id === '') {
      findings.push({
        path: MANIFEST_PATH,
        severity: 'error',
        category: 'invalid-manifest-entry',
        message: `scenario entry (world "${entry.world ?? ''}") has an empty id`,
      });
    }
    if (world === '') {
      findings.push({
        path: MANIFEST_PATH,
        severity: 'error',
        category: 'invalid-manifest-entry',
        message: `scenario "${entry.id ?? ''}" has an empty world path`,
      });
      continue;
    }

    if (id !== '') {
      if (seenIds.has(id)) {
        findings.push({
          path: MANIFEST_PATH,
          severity: 'error',
          category: 'duplicate-scenario-id',
          message: `scenario id "${id}" is declared more than once`,
        });
      } else {
        seenIds.add(id);
      }
    }

    if (!isWorldContentPath(world)) {
      findings.push({
        path: MANIFEST_PATH,
        severity: 'error',
        category: 'invalid-manifest-world-path',
        message: `scenario "${id}" world "${world}" is not a supported assets/worlds/*.toml path`,
      });
      continue;
    }

    const worldToml = contentByPath[world];
    if (worldToml === undefined) {
      findings.push({
        path: MANIFEST_PATH,
        severity: 'error',
        category: 'missing-scenario-world',
        message: `scenario "${id}" references world "${world}" which is not included in the pack`,
      });
      continue;
    }
    try {
      tomlParse(worldToml);
    } catch (e) {
      findings.push({
        path: MANIFEST_PATH,
        severity: 'error',
        category: 'unparseable-scenario-world',
        message: `scenario "${id}" world "${world}" failed to parse: ${e.message}`,
      });
    }
  }

  return findings;
}

/**
 * Export a validated TOML mod pack.
 *
 * @param {{
 *   files: Array<{ path: string, parsed: object, text?: string }>,
 *   scenarios: Array<{ id: string, world: string, label?: string }>,
 *   rigIndex?: import('./marker-validate.js').RigIndex,
 * }} input
 *   `files` are the selected authored files (parsed TOML plus optional
 *   pre-serialised `text`; when absent the parsed object is serialised with
 *   smol-toml). `scenarios` are the manifest's root-world entries. `rigIndex`,
 *   when supplied, drives cross-file marker validation exactly as a save does.
 *
 * @returns {{ ok: true, zip: Uint8Array, manifestToml: string,
 *             paths: string[], warnings: string[] }
 *          | { ok: false, errors: string[], warnings: string[] }}
 *
 * The gate mirrors `SaveFlow._saveOne` (issue #757) but across every selected
 * file AND the manifest: definite errors block the whole export before any
 * archive byte is produced; warnings are surfaced but never block.
 */
export function exportModPack(input) {
  const files = Array.isArray(input?.files) ? input.files : [];
  const scenarios = Array.isArray(input?.scenarios) ? input.scenarios : [];
  const rigIndex = input?.rigIndex ?? null;

  const errors = [];
  const warnings = [];

  // 1. Path whitelist — a file that escapes the supported authored paths is a
  //    definite error, never silently dropped.
  const contentByPath = {};
  const zipEntries = [];
  const seenPaths = new Set();
  for (const file of files) {
    const path = file?.path;
    if (file?.path === MANIFEST_PATH) {
      errors.push(
        `"${MANIFEST_PATH}" is generated by the exporter and must not be a selected file`,
      );
      continue;
    }
    if (!isAllowedContentPath(path)) {
      errors.push(`"${path}" is not a supported authored TOML path and cannot be exported`);
      continue;
    }
    if (seenPaths.has(path)) {
      errors.push(`"${path}" is selected more than once`);
      continue;
    }
    seenPaths.add(path);

    // 2. Per-file admission — the same validateFile + partitionFindings gate a
    //    save uses, applied to EVERY selected file.
    let text;
    try {
      text = typeof file.text === 'string' ? file.text : tomlStringify(file.parsed);
    } catch (e) {
      errors.push(`"${path}" could not be serialised: ${e.message}`);
      continue;
    }

    const findings = validateFile(path, file.parsed, { rigIndex });
    const { errors: fileErrors, warnings: fileWarnings } = partitionFindings(findings);
    for (const r of fileErrors) errors.push(`${path}: ${r.message}`);
    for (const r of fileWarnings) warnings.push(`${path}: ${r.message}`);

    contentByPath[path] = text;
    zipEntries.push({ path, text });
  }

  // 3. Manifest root-world validation against the selected content.
  const manifestFindings = validateManifestEntries(scenarios, contentByPath);
  const { errors: manifestErrors, warnings: manifestWarnings } =
    partitionFindings(manifestFindings);
  for (const r of manifestErrors) errors.push(`${MANIFEST_PATH}: ${r.message}`);
  for (const r of manifestWarnings) warnings.push(`${MANIFEST_PATH}: ${r.message}`);

  if (errors.length > 0) {
    return { ok: false, errors, warnings };
  }

  // 4. Build the archive: the required manifest plus every validated file.
  const manifestToml = buildManifestToml(scenarios);
  const archiveEntries = [{ path: MANIFEST_PATH, text: manifestToml }, ...zipEntries];
  const zip = createStoreZip(archiveEntries);

  return {
    ok: true,
    zip,
    manifestToml,
    paths: archiveEntries.map((e) => e.path),
    warnings,
  };
}
