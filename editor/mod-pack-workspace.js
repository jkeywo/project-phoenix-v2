/**
 * mod-pack-workspace.js — PURE mod-pack authoring workspace (issue #989).
 *
 * No DOM, no file IO — the same discipline as `mod-pack-export.js`. This module
 * is the model behind MOD mode: it holds the pack's `[pack]` identity metadata,
 * its `[[scenario]]` manifest entries, and its member set, and it classifies
 * each member as `new` or `patch` against a supplied base-file map (the
 * project's on-disk files), recording the base file's digest at add-time so a
 * later export can WARN (never block) when the base drifted since it was added.
 *
 * The view (`mod-mode-view.js`) owns all IO: it reads the on-disk base files to
 * build the base-file map this workspace classifies against, resolves a composed
 * hull's fragment closure into extra members (#910), reads back an imported ZIP,
 * and drives export/import through `mod-pack-export.js`. This module only records
 * and reports — every method is synchronous and side-effect-free.
 */

import { parse as tomlParse } from 'smol-toml';
import { crc32, PACK_FORMAT, MANIFEST_PATH, parsePackManifest } from './mod-pack-export.js';

const encoder = new TextEncoder();

const str = (v) => (typeof v === 'string' ? v : v == null ? '' : String(v));

/**
 * Content digest used to detect base-file drift for the stale-patch warning.
 * CRC-32 (hex) is a checksum, not a security boundary — enough to notice the
 * base a patch was derived from has changed since it was recorded. Reuses the
 * exporter's CRC so both agree on one hashing implementation.
 */
export function digestText(text) {
  return crc32(encoder.encode(str(text))).toString(16).padStart(8, '0');
}

/**
 * Classify a member path against the supplied base-file map. A path present in
 * the map — i.e. a file that already exists under the project root — is a
 * `patch` (an override of shipped content); a path the map does not carry is
 * `new` (an asset the pack introduces).
 */
export function classifyMember(path, baseFiles = {}) {
  return baseFiles && Object.prototype.hasOwnProperty.call(baseFiles, path)
    ? 'patch'
    : 'new';
}

/** Normalise caller-supplied `[pack]` metadata into the workspace's own shape.
 * `format` defaults to {@link PACK_FORMAT}; string fields coerce to strings;
 * `content_epoch` is preserved as-is (or `null` when unset) so a genuinely
 * missing epoch is still caught by the exporter's `validatePackMeta`. */
function normalisePackMeta(pack) {
  const p = pack || {};
  const req = p.requires || {};
  return {
    format: Number.isInteger(p.format) ? p.format : PACK_FORMAT,
    id: str(p.id),
    version: str(p.version),
    name: str(p.name),
    author: str(p.author),
    description: str(p.description),
    requires: {
      content_id: str(req.content_id),
      content_epoch: req.content_epoch == null ? null : req.content_epoch,
    },
  };
}

export class ModPackWorkspace {
  constructor(initial = {}) {
    this._pack = normalisePackMeta(initial.pack);
    /** @type {Array<{id:string, world:string, label?:string}>} */
    this._scenarios = [];
    /** @type {Map<string,{path:string,text:string,classification:string,baseDigest:string|null}>} */
    this._members = new Map();

    if (Array.isArray(initial.scenarios)) {
      for (const s of initial.scenarios) this._appendScenario(s);
    }
    if (Array.isArray(initial.members)) {
      for (const m of initial.members) {
        if (!m || typeof m.path !== 'string' || m.path.length === 0) continue;
        this._members.set(m.path, {
          path: m.path,
          text: str(m.text),
          classification: m.classification === 'patch' ? 'patch' : 'new',
          baseDigest: m.baseDigest ?? null,
        });
      }
    }
  }

  // ── Pack identity ([pack] header) ─────────────────────────────────────────

  getPack() {
    return { ...this._pack, requires: { ...this._pack.requires } };
  }

  /**
   * Merge a partial `[pack]` patch into the current metadata. A field present
   * in `patch` (including an empty string, so a cleared input takes) overwrites;
   * an OMITTED field (`undefined`) is left unchanged.
   */
  setPack(patch = {}) {
    const cur = this._pack;
    const req = patch.requires || {};
    this._pack = normalisePackMeta({
      format: patch.format === undefined ? cur.format : patch.format,
      id: patch.id === undefined ? cur.id : patch.id,
      version: patch.version === undefined ? cur.version : patch.version,
      name: patch.name === undefined ? cur.name : patch.name,
      author: patch.author === undefined ? cur.author : patch.author,
      description: patch.description === undefined ? cur.description : patch.description,
      requires: {
        content_id:
          req.content_id === undefined ? cur.requires.content_id : req.content_id,
        content_epoch:
          req.content_epoch === undefined ? cur.requires.content_epoch : req.content_epoch,
      },
    });
    return this.getPack();
  }

  // ── Scenarios ([[scenario]] entries) ──────────────────────────────────────

  _appendScenario(s) {
    const entry = { id: str(s?.id), world: str(s?.world) };
    if (typeof s?.label === 'string' && s.label.length > 0) entry.label = s.label;
    this._scenarios.push(entry);
  }

  getScenarios() {
    return this._scenarios.map((s) => ({ ...s }));
  }

  setScenarios(list) {
    this._scenarios = [];
    if (Array.isArray(list)) for (const s of list) this._appendScenario(s);
    return this.getScenarios();
  }

  addScenario(entry) {
    this._appendScenario(entry || {});
    return this.getScenarios();
  }

  removeScenario(id) {
    const before = this._scenarios.length;
    this._scenarios = this._scenarios.filter((s) => s.id !== id);
    return this._scenarios.length !== before;
  }

  // ── Members ───────────────────────────────────────────────────────────────

  /**
   * Add (or replace) a member. `member.text` is the content the pack carries
   * verbatim; classification is decided against `baseFiles`. A `patch` records
   * the digest of its base at add-time so {@link staleWarnings} can later notice
   * the base drifted; a `new` member records no base digest.
   */
  addMember(member, baseFiles = {}) {
    const path = member?.path;
    if (typeof path !== 'string' || path.length === 0) {
      throw new Error('a mod-pack member requires a non-empty path');
    }
    const text = str(member.text);
    const classification = classifyMember(path, baseFiles);
    const baseDigest = classification === 'patch' ? digestText(baseFiles[path]) : null;
    const record = { path, text, classification, baseDigest };
    this._members.set(path, record);
    return { ...record };
  }

  removeMember(path) {
    return this._members.delete(path);
  }

  hasMember(path) {
    return this._members.has(path);
  }

  getMember(path) {
    const m = this._members.get(path);
    return m ? { ...m } : null;
  }

  /** Members in insertion order (which is archive order after an import). */
  getMembers() {
    return [...this._members.values()].map((m) => ({ ...m }));
  }

  memberCount() {
    return this._members.size;
  }

  // ── Stale-patch provenance ────────────────────────────────────────────────

  /**
   * Non-blocking WARNINGS for every `patch` member whose base file has changed
   * on disk since it was added (its current digest differs from the one recorded
   * at add-time). `currentBaseFiles` is the freshly re-read `{ path: text }` map.
   * A patch whose base has vanished from the map is left alone — its absence is
   * a separate concern, not drift.
   */
  staleWarnings(currentBaseFiles = {}) {
    const out = [];
    for (const m of this._members.values()) {
      if (m.classification !== 'patch') continue;
      if (!Object.prototype.hasOwnProperty.call(currentBaseFiles, m.path)) continue;
      const now = digestText(currentBaseFiles[m.path]);
      if (now !== m.baseDigest) {
        out.push({
          path: m.path,
          severity: 'warning',
          category: 'stale-patch',
          message: `patch "${m.path}" was derived from a base file that has since changed — re-check the override before shipping`,
        });
      }
    }
    return out;
  }

  // ── Export bridge ─────────────────────────────────────────────────────────

  /**
   * Build the input `exportModPack` consumes. Every member rides along verbatim
   * as `text`; TOML members are additionally parsed so the export gate can
   * validate them (a composed hull resolves against its fragment members). A
   * member that will not parse is passed WITHOUT `parsed`, so the export gate
   * reports it rather than this pure module throwing.
   */
  toExportInput() {
    const files = [];
    for (const m of this._members.values()) {
      if (m.path.endsWith('.rhai')) {
        files.push({ path: m.path, text: m.text });
        continue;
      }
      let parsed;
      try {
        parsed = tomlParse(m.text);
      } catch {
        parsed = undefined;
      }
      files.push({ path: m.path, text: m.text, parsed });
    }
    return { pack: this.getPack(), scenarios: this.getScenarios(), files };
  }

  // ── Import ────────────────────────────────────────────────────────────────

  /**
   * Rebuild a workspace from a decoded archive (`readStoreZip` output — a
   * `{ path: text }` map). The manifest seeds pack metadata + scenarios; every
   * other entry becomes a member, classified against `baseFiles`. Members keep
   * the archive's insertion order, so re-exporting an unedited import produces
   * BYTE-IDENTICAL archive bytes.
   */
  static fromArchiveFiles(files, baseFiles = {}) {
    const manifestText = files && files[MANIFEST_PATH];
    const { pack, scenarios } = manifestText
      ? parsePackManifest(manifestText)
      : { pack: null, scenarios: [] };
    const ws = new ModPackWorkspace({ pack, scenarios });
    for (const path of Object.keys(files || {})) {
      if (path === MANIFEST_PATH) continue;
      ws.addMember({ path, text: files[path] }, baseFiles);
    }
    return ws;
  }
}
