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
 *
 * ── Rhai scripts: a deliberate JS/host asymmetry (issue #988) ────────────────
 *
 * Every other check in this file MIRRORS a Rust validator, so the editor refuses
 * locally exactly what the host would refuse on upload. `.rhai` members are the
 * ONE exception. This exporter does only STRUCTURAL checks on a script — its
 * path is a sibling `assets/worlds/*.rhai`, its text is non-empty, and some
 * world's `script = "..."` references it — and CANNOT compile Rhai. Whether a
 * script is *safe* is decided solely by the host's deny-by-default vellum
 * sandbox (`validate_pack_scripts` in src/world/mod_pack.rs), which compiles
 * every script and rejects any that fails to compile or reaches for a denied
 * capability (eval, import/module resolve, wall clock). The file extension is
 * not the trust boundary; the sandbox is. So a `.rhai` this exporter admits may
 * still be rejected on upload — and that is the host's call to make, not the
 * editor's.
 */

import { stringify as tomlStringify, parse as tomlParse } from 'smol-toml';
import { validateFile, partitionFindings } from './validation.js';
import { resolveTemplate, canonicalTemplatePath, INCLUDES_KEY } from './entity-includes.js';

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
 * file must sit directly under a supported `assets/*` directory, carry a
 * non-empty file name with no path traversal.
 *
 * A supported authored file is a `.toml` under one of {@link ALLOWED_DIR_PREFIXES},
 * OR a `.rhai` script directly under `assets/worlds/` (issue #988) — the sibling
 * layout `world::script::load` resolves a world's `script = "..."` to. The
 * extension is NOT the trust boundary: the host's deny-by-default sandbox
 * compiles and gates the script (see the module header). Mirrors
 * `is_allowed_content_path` in `src/world/mod_pack.rs`.
 */
export function isAllowedContentPath(path) {
  if (typeof path !== 'string' || path.length === 0) return false;
  if (path.includes('..') || path.includes('\\')) return false;
  if (path === MANIFEST_PATH) return true;
  // Rhai scripts sit beside the world that loads them: a sibling
  // assets/worlds/*.rhai, and nowhere else.
  if (path.endsWith('.rhai')) {
    if (!path.startsWith('assets/worlds/')) return false;
    const name = path.slice('assets/worlds/'.length);
    return name.length > '.rhai'.length && !name.includes('/');
  }
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

/**
 * Resolve a world's sibling script path, mirroring `sibling_path` in
 * `src/world/script/load.rs`: forward slashes, relative to the world file's
 * directory. Used only for the referenced-by-a-world check on `.rhai` members.
 */
export function siblingScriptPath(worldPath, rel) {
  const r = String(rel).replace(/\\/g, '/');
  const i = Math.max(worldPath.lastIndexOf('/'), worldPath.lastIndexOf('\\'));
  return i >= 0 ? `${worldPath.slice(0, i).replace(/\\/g, '/')}/${r}` : r;
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

/** The `[pack]` manifest format this exporter emits (issue #986). Mirrors
 * `SUPPORTED_PACK_FORMAT` in `src/world/manifest.rs`: a versioning constant, not
 * a gameplay value. */
export const PACK_FORMAT = 1;

/**
 * Normalise the caller-supplied `pack` metadata into the `[pack]` table shape
 * `parse_pack_manifest` reads (`src/world/manifest.rs`). `format` defaults to
 * {@link PACK_FORMAT}; `author`/`description` are emitted only when present;
 * `requires` always carries `content_id` + `content_epoch`.
 */
export function buildPackTable(pack) {
  const p = {
    format: Number.isInteger(pack?.format) ? pack.format : PACK_FORMAT,
    id: String(pack?.id ?? ''),
    version: String(pack?.version ?? ''),
    name: String(pack?.name ?? ''),
  };
  if (typeof pack?.author === 'string' && pack.author.length > 0) p.author = pack.author;
  if (typeof pack?.description === 'string' && pack.description.length > 0) {
    p.description = pack.description;
  }
  const req = pack?.requires ?? {};
  p.requires = {
    content_id: String(req.content_id ?? ''),
    content_epoch: Number(req.content_epoch ?? 0),
  };
  return p;
}

/**
 * Serialise a mod-pack manifest to TOML. When `pack` metadata is supplied the
 * `[pack]` identity table (issue #986) is emitted ABOVE the `[[scenario]]`
 * entries; each scenario emits `id`, `world`, and an optional `label`, matching
 * the schema `parse_pack_manifest`/`parse_manifest` read (`src/world/manifest.rs`).
 */
export function buildManifestToml(scenarios, pack) {
  const scenario = [];
  for (const s of scenarios || []) {
    const entry = { id: String(s.id ?? ''), world: String(s.world ?? '') };
    if (typeof s.label === 'string' && s.label.length > 0) entry.label = s.label;
    scenario.push(entry);
  }
  // Object key order is the emit order: `[pack]` (and its nested
  // `[pack.requires]`) precede `[[scenario]]`.
  const doc = pack ? { pack: buildPackTable(pack), scenario } : { scenario };
  return tomlStringify(doc);
}

/**
 * The findings that block export of a pack with missing/invalid `[pack]`
 * metadata (issue #986). Returned as message strings so `exportModPack` can
 * fold them into its error list. A pack MUST declare an id, version, name, and a
 * `requires` clause naming the content id + epoch it was authored against.
 */
export function validatePackMeta(pack) {
  const errors = [];
  if (!pack || typeof pack !== 'object') {
    errors.push('a mod pack requires [pack] metadata (id, version, name, requires) — none was provided');
    return errors;
  }
  if (String(pack.id ?? '').trim() === '') errors.push('[pack] id is required and must not be empty');
  if (String(pack.version ?? '').trim() === '') errors.push('[pack] version is required');
  if (String(pack.name ?? '').trim() === '') errors.push('[pack] name is required');
  const req = pack.requires;
  if (!req || typeof req !== 'object') {
    errors.push('[pack.requires] is required — name the content_id + content_epoch the pack targets');
  } else {
    if (String(req.content_id ?? '').trim() === '') {
      errors.push('[pack.requires] content_id is required');
    }
    if (!Number.isInteger(req.content_epoch)) {
      errors.push('[pack.requires] content_epoch is required and must be an integer');
    }
  }
  return errors;
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

/** Normalise the fragment-text source into a `read(path) -> string | null`. */
function normaliseFragmentSource(fs) {
  if (!fs) return () => null;
  if (typeof fs === 'function') return (p) => fs(p) ?? null;
  if (typeof fs.read === 'function') return (p) => fs.read(p) ?? null;
  if (fs instanceof Map) return (p) => (fs.has(p) ? fs.get(p) : null);
  if (typeof fs === 'object') return (p) => (Object.prototype.hasOwnProperty.call(fs, p) ? fs[p] : null);
  return () => null;
}

function declaresIncludes(parsed) {
  return (
    parsed !== null &&
    typeof parsed === 'object' &&
    !Array.isArray(parsed) &&
    Object.prototype.hasOwnProperty.call(parsed, INCLUDES_KEY)
  );
}

/**
 * Export a validated TOML mod pack.
 *
 * @param {{
 *   files: Array<{ path: string, parsed: object, text?: string }>,
 *   scenarios: Array<{ id: string, world: string, label?: string }>,
 *   pack: { format?: number, id: string, version: string, name: string,
 *           author?: string, description?: string,
 *           requires: { content_id: string, content_epoch: number } },
 *   rigIndex?: import('./marker-validate.js').RigIndex,
 *   fragmentSource?: Function | {read:Function} | Map | object,
 * }} input
 *   `pack` is the required `[pack]` identity metadata (issue #986); an export
 *   without valid pack metadata is refused. `files` are the selected authored
 *   files (parsed TOML plus optional
 *   pre-serialised `text`; when absent the parsed object is serialised with
 *   smol-toml). `scenarios` are the manifest's root-world entries. `rigIndex`,
 *   when supplied, drives cross-file marker validation exactly as a save does.
 *   `fragmentSource`, when supplied, serves raw TOML text for `includes`
 *   fragments a composed hull depends on that were not themselves selected — so
 *   the exporter can carry them into the pack (issue #910).
 *
 * @returns {{ ok: true, zip: Uint8Array, manifestToml: string,
 *             paths: string[], warnings: string[] }
 *          | { ok: false, errors: string[], warnings: string[] }}
 *
 * The gate mirrors `SaveFlow._saveOne` (issue #757) but across every selected
 * file AND the manifest: definite errors block the whole export before any
 * archive byte is produced; warnings are surfaced but never block.
 *
 * A composed hull (issue #910) is validated as its RESOLVED document — so a
 * hull whose systems come from a fragment does not read as having none — and
 * every fragment in its include closure is CARRIED into the pack as a
 * dependency. An include that cannot be resolved (missing fragment, cycle,
 * malformed declaration) blocks the export with an error naming the declaring
 * hull, so an exported pack never references a fragment it lacks.
 */
export function exportModPack(input) {
  const files = Array.isArray(input?.files) ? input.files : [];
  const scenarios = Array.isArray(input?.scenarios) ? input.scenarios : [];
  const pack = input?.pack ?? null;
  const rigIndex = input?.rigIndex ?? null;
  const fragmentSource = normaliseFragmentSource(input?.fragmentSource);

  const errors = [];
  const warnings = [];

  // 0. Pack identity is required (issue #986): a pack without a valid [pack]
  //    header cannot be uploaded (the host rejects `missing-pack-header`), so
  //    the exporter refuses to produce one.
  for (const e of validatePackMeta(pack)) errors.push(e);

  // 1. Path whitelist + serialisation — a file that escapes the supported
  //    authored paths is a definite error, never silently dropped. Validation
  //    is deferred to step 2 so composed hulls can be validated RESOLVED.
  const contentByPath = {};
  const parsedByPath = {};
  const scriptPaths = [];
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
      errors.push(`"${path}" is not a supported authored path and cannot be exported`);
      continue;
    }
    if (seenPaths.has(path)) {
      errors.push(`"${path}" is selected more than once`);
      continue;
    }

    // A `.rhai` script is raw text, not TOML: it is carried verbatim and gated
    // by the host's deny-by-default sandbox on upload (issue #988). The exporter
    // checks only that it is non-empty here; the referenced-by-a-world check
    // runs in step 1a. It cannot — and must not — try to compile Rhai; the host
    // is the authoritative gate (see the module header).
    if (path.endsWith('.rhai')) {
      const src =
        typeof file.text === 'string'
          ? file.text
          : typeof file.parsed === 'string'
            ? file.parsed
            : '';
      if (src.trim().length === 0) {
        errors.push(`"${path}" is an empty Rhai script and cannot be exported`);
        continue;
      }
      seenPaths.add(path);
      scriptPaths.push(path);
      zipEntries.push({ path, text: src });
      continue;
    }

    let text;
    try {
      text = typeof file.text === 'string' ? file.text : tomlStringify(file.parsed);
    } catch (e) {
      errors.push(`"${path}" could not be serialised: ${e.message}`);
      continue;
    }

    seenPaths.add(path);
    contentByPath[path] = text;
    parsedByPath[path] = file.parsed;
    zipEntries.push({ path, text });
  }

  // 1a. Every `.rhai` member must be referenced by a world's `script = "..."`
  //     (issue #988), so an exported pack never carries an orphan script. Only a
  //     string `script` names a sibling file; an inline [script] table is
  //     embedded in the world TOML, not a separate member.
  const referencedScripts = new Set();
  for (const [worldPath, parsed] of Object.entries(parsedByPath)) {
    if (!worldPath.startsWith('assets/worlds/') || !worldPath.endsWith('.toml')) continue;
    const s = parsed && typeof parsed === 'object' ? parsed.script : undefined;
    if (typeof s === 'string' && s.length > 0) {
      referencedScripts.add(siblingScriptPath(worldPath, s));
    }
  }
  for (const sp of scriptPaths) {
    if (!referencedScripts.has(sp)) {
      errors.push(
        `"${sp}" is not referenced by any world's \`script = "..."\` and cannot be exported`,
      );
    }
  }

  // Resolution reads a composed hull's own text and its fragments' text: the
  // selected files first, then the caller-supplied fragment source.
  const read = (p) =>
    Object.prototype.hasOwnProperty.call(contentByPath, p) ? contentByPath[p] : fragmentSource(p);

  // Carry one fragment dependency into the pack (verbatim — a fragment is a
  // partial and must not be validated standalone, only through the resolved
  // hull that composes it).
  const carryFragment = (fragmentPath, declaringHull) => {
    if (seenPaths.has(fragmentPath)) return; // already in the pack
    if (!isAllowedContentPath(fragmentPath)) {
      errors.push(
        `${declaringHull}: include "${fragmentPath}" is not a supported authored TOML path and cannot be exported`,
      );
      return;
    }
    const text = read(fragmentPath);
    if (text == null) {
      errors.push(`${declaringHull}: fragment "${fragmentPath}" is not included in the pack`);
      return;
    }
    seenPaths.add(fragmentPath);
    contentByPath[fragmentPath] = text;
    zipEntries.push({ path: fragmentPath, text });
  };

  // 2. Per-file admission — the same validateFile + partitionFindings gate a
  //    save uses, applied to EVERY selected file. A composed hull resolves
  //    first, so it is validated RESOLVED and its fragments are carried.
  for (const path of Object.keys(parsedByPath)) {
    const parsed = parsedByPath[path];
    let toValidate = parsed;

    if (path.startsWith('assets/entities/') && declaresIncludes(parsed)) {
      const result = resolveTemplate(path, read, tomlParse);
      if (!result.ok) {
        const e = result.error;
        errors.push(
          `${e.file}: ${e.category}: ${e.message} [include chain: ${e.chain.join(' -> ')}]`,
        );
        continue; // a hull that will not resolve cannot be validated or carried
      }
      toValidate = result.resolved.value;
      const root = canonicalTemplatePath(path);
      for (const src of result.resolved.sources) {
        if (src === root) continue;
        carryFragment(src, path);
      }
    }

    const findings = validateFile(path, toValidate, { rigIndex });
    const { errors: fileErrors, warnings: fileWarnings } = partitionFindings(findings);
    for (const r of fileErrors) errors.push(`${path}: ${r.message}`);
    for (const r of fileWarnings) warnings.push(`${path}: ${r.message}`);
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

  // 4. Build the archive: the required manifest (with its [pack] header) plus
  //    every validated file.
  const manifestToml = buildManifestToml(scenarios, pack);
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
