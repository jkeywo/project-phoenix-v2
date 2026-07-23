/**
 * models-rig.js
 *
 * PURE logic for the Models Mode rig sidecar. No DOM, no Three.js — every
 * function here is unit-testable in a plain node environment.
 *
 * A "rig" corrects a raw GLB into game space and pins named marker points
 * onto the post-base-rig model. It is serialized as a TOML sidecar next to
 * the .glb in assets/models/:
 *
 *   - Default/base variant:  <stem>.model.toml      ("model" is reserved)
 *   - Named variant:         <stem>.<variant>.toml
 *
 * Schema (EXACT):
 *
 *   [base]                       # applied INNER, before per-entity transform
 *   offset   = [0.0, 0.0, 0.0]   # non-uniform vec3
 *   rotation = [0.0, 0.0, 0.0]   # XYZ-order Euler radians (x,y,z), matching the
 *                                # engine's Quat::from_euler(EulerRot::XYZ, ...)
 *   scale    = [1.0, 1.0, 1.0]   # non-uniform vec3
 *
 *   [extents]                    # cached, recomputed on save, POST-base-rig
 *   min  = [-4.0, -1.2, -6.0]
 *   max  = [ 4.0,  1.2,  6.0]
 *   size = [ 8.0,  2.4, 12.0]
 *
 *   [markers]                    # one [markers.<name>] sub-table per marker
 *   [markers.fore_emitter]
 *   position  = [0.0, 0.0, -6.0]
 *   direction = [0.0, 0.0, -1.0]
 *
 * A marker's `direction` is a unit vector in post-base-rig space. The
 * forward basis is (0,0,-1) (game -Z forward). Roll is intentionally
 * dropped: only the resulting forward direction is serialized.
 */
import { parse, stringify } from 'smol-toml';
import { validateRigMarkerNames, validateRigSidecarToml } from './marker-validate.js';

/** Reserved variant name for the default/base sidecar. */
export const DEFAULT_VARIANT = 'model';

/** Game forward basis: -Z. */
export const FORWARD = Object.freeze([0, 0, -1]);

// ── vec3 helpers ────────────────────────────────────────────────────────

/**
 * Coerce an arbitrary value into a numeric vec3, falling back to `fallback`
 * (component-wise) for anything non-finite.
 */
export function toVec3(value, fallback = [0, 0, 0]) {
  const out = [fallback[0], fallback[1], fallback[2]];
  if (Array.isArray(value)) {
    for (let i = 0; i < 3; i++) {
      const n = Number(value[i]);
      if (Number.isFinite(n)) out[i] = n;
    }
  }
  return out;
}

/** Euclidean length of a vec3. */
export function vec3Length(v) {
  return Math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
}

/**
 * Normalize a vec3 to unit length. A zero-length (or non-finite) vector
 * falls back to the game forward basis (0,0,-1).
 */
export function normalizeDirection(v) {
  const vec = toVec3(v, FORWARD);
  const len = vec3Length(vec);
  if (!Number.isFinite(len) || len < 1e-9) {
    return [...FORWARD];
  }
  return [vec[0] / len, vec[1] / len, vec[2] / len];
}

// ── default rig ─────────────────────────────────────────────────────────

/** A fresh, identity rig with no markers and zero extents. */
export function defaultRig() {
  return {
    base: {
      offset: [0, 0, 0],
      rotation: [0, 0, 0],
      scale: [1, 1, 1],
    },
    extents: {
      min: [0, 0, 0],
      max: [0, 0, 0],
      size: [0, 0, 0],
    },
    markers: {},
  };
}

// ── extents ─────────────────────────────────────────────────────────────

/**
 * Compute cached extents from a bounding box. Accepts `{ min, max }` vec3s
 * (in post-base-rig space) and returns `{ min, max, size }` where
 * `size = max - min` component-wise.
 */
export function computeExtents({ min, max } = {}) {
  const lo = toVec3(min, [0, 0, 0]);
  const hi = toVec3(max, [0, 0, 0]);
  return {
    min: lo,
    max: hi,
    size: [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
  };
}

// ── marker CRUD (operate on a plain rig object, returning the same rig) ──

/**
 * Add (or replace) a marker. Position/direction are coerced; direction is
 * normalized to unit length. Returns the rig for chaining. Throws on an
 * empty/invalid name.
 */
export function addMarker(rig, name, { position, direction } = {}) {
  const key = normalizeMarkerName(name);
  rig.markers[key] = {
    position: toVec3(position, [0, 0, 0]),
    direction: normalizeDirection(direction),
  };
  return rig;
}

/** Update an existing marker's position and/or direction in place. */
export function updateMarker(rig, name, { position, direction } = {}) {
  const existing = rig.markers[name];
  if (!existing) return rig;
  if (position !== undefined) existing.position = toVec3(position, existing.position);
  if (direction !== undefined) existing.direction = normalizeDirection(direction);
  return rig;
}

/** Remove a marker by name. No-op if it doesn't exist. */
export function removeMarker(rig, name) {
  delete rig.markers[name];
  return rig;
}

/**
 * Rename a marker, preserving its position/direction and (best-effort) key
 * ordering. Throws if the new name is empty/invalid or already taken by a
 * different marker. No-op if `from === to`.
 */
export function renameMarker(rig, from, to) {
  const newKey = normalizeMarkerName(to);
  if (from === newKey) return rig;
  if (!(from in rig.markers)) return rig;
  if (newKey in rig.markers) {
    throw new Error(`Marker "${newKey}" already exists`);
  }
  // Rebuild to keep insertion order stable (renamed key takes old slot).
  const rebuilt = {};
  for (const [k, v] of Object.entries(rig.markers)) {
    rebuilt[k === from ? newKey : k] = v;
  }
  rig.markers = rebuilt;
  return rig;
}

function normalizeMarkerName(name) {
  const trimmed = typeof name === 'string' ? name.trim() : '';
  if (!trimmed) throw new Error('Marker name must be a non-empty string');
  return trimmed;
}

// ── TOML parse / build ──────────────────────────────────────────────────

/**
 * Parse a sidecar TOML string into a normalized rig object. Missing
 * sections fall back to defaults; markers are coerced and their directions
 * re-normalized so callers always get a well-formed rig.
 */
export function parseRigToml(text) {
  const parsed = parse(text);
  const rig = defaultRig();

  const base = parsed.base || {};
  rig.base.offset = toVec3(base.offset, [0, 0, 0]);
  rig.base.rotation = toVec3(base.rotation, [0, 0, 0]);
  rig.base.scale = toVec3(base.scale, [1, 1, 1]);

  const extents = parsed.extents || {};
  rig.extents = computeExtentsFromStored(extents);

  const markers = parsed.markers || {};
  for (const [name, m] of Object.entries(markers)) {
    if (!m || typeof m !== 'object') continue;
    rig.markers[name] = {
      position: toVec3(m.position, [0, 0, 0]),
      direction: normalizeDirection(m.direction),
    };
  }

  return rig;
}

/**
 * When reading a sidecar we trust stored min/max but recompute `size` so a
 * hand-edited file can't drift. If min/max are absent, everything is zero.
 */
function computeExtentsFromStored(extents) {
  return computeExtents({ min: extents.min, max: extents.max });
}

/**
 * Serialize a rig object to a TOML sidecar string. The emitted order is
 * [base] → [extents] → [markers]; smol-toml emits each marker as a
 * `[markers.<name>]` sub-table (NOT an inline table), which round-trips
 * cleanly back through parseRigToml.
 */
export function buildRigToml(rig) {
  const safe = rig || defaultRig();
  const base = safe.base || {};
  const extents = safe.extents || {};
  const markers = safe.markers || {};

  const doc = {
    base: {
      offset: toVec3(base.offset, [0, 0, 0]),
      rotation: toVec3(base.rotation, [0, 0, 0]),
      scale: toVec3(base.scale, [1, 1, 1]),
    },
    extents: {
      min: toVec3(extents.min, [0, 0, 0]),
      max: toVec3(extents.max, [0, 0, 0]),
      size: toVec3(extents.size, [0, 0, 0]),
    },
    markers: {},
  };

  for (const [name, m] of Object.entries(markers)) {
    doc.markers[name] = {
      position: toVec3(m.position, [0, 0, 0]),
      direction: normalizeDirection(m.direction),
    };
  }

  return stringify(doc);
}

// ── variant filename helpers ────────────────────────────────────────────

/**
 * Build the sidecar filename for a model stem + variant.
 *   buildSidecarName('asteroid_large', 'model')     -> 'asteroid_large.model.toml'
 *   buildSidecarName('asteroid_large', 'weathered') -> 'asteroid_large.weathered.toml'
 * A missing/empty variant defaults to the reserved 'model' name.
 */
export function buildSidecarName(stem, variant = DEFAULT_VARIANT) {
  const v = typeof variant === 'string' && variant.trim() ? variant.trim() : DEFAULT_VARIANT;
  return `${stem}.${v}.toml`;
}

/**
 * Parse a sidecar filename back into `{ stem, variant }`, or null if it
 * isn't a `<stem>.<variant>.toml` sidecar.
 *   parseSidecarName('asteroid_large.model.toml')     -> { stem: 'asteroid_large', variant: 'model' }
 *   parseSidecarName('asteroid_large.weathered.toml') -> { stem: 'asteroid_large', variant: 'weathered' }
 *   parseSidecarName('asteroid_large.glb')            -> null
 */
export function parseSidecarName(filename) {
  if (typeof filename !== 'string') return null;
  if (!filename.endsWith('.toml')) return null;
  const withoutExt = filename.slice(0, -'.toml'.length);
  const lastDot = withoutExt.lastIndexOf('.');
  if (lastDot <= 0) return null; // need a stem AND a variant segment
  const stem = withoutExt.slice(0, lastDot);
  const variant = withoutExt.slice(lastDot + 1);
  if (!stem || !variant) return null;
  return { stem, variant };
}

/**
 * Derive the model stem from a .glb filename.
 *   glbStem('asteroid_large.glb') -> 'asteroid_large'
 * Returns null for non-.glb names.
 */
export function glbStem(filename) {
  if (typeof filename !== 'string' || !filename.toLowerCase().endsWith('.glb')) return null;
  return filename.slice(0, -'.glb'.length);
}

/**
 * Group a directory listing of assets/models into models and their
 * variants. Accepts the raw `[{ name, kind }]` from listDirectory and
 * returns `[{ stem, glb, variants: string[] }]` sorted by stem, where
 * `variants` is the sorted list of sidecar variant names found for that
 * stem (the reserved 'model' default sorted first if present).
 */
export function groupModelFiles(entries) {
  const glbs = new Map(); // stem -> glb filename
  const variantsByStem = new Map(); // stem -> Set<variant>

  for (const entry of entries || []) {
    if (!entry || entry.kind !== 'file' || typeof entry.name !== 'string') continue;
    const stem = glbStem(entry.name);
    if (stem) {
      glbs.set(stem, entry.name);
      continue;
    }
    const sidecar = parseSidecarName(entry.name);
    if (sidecar) {
      if (!variantsByStem.has(sidecar.stem)) variantsByStem.set(sidecar.stem, new Set());
      variantsByStem.get(sidecar.stem).add(sidecar.variant);
    }
  }

  const result = [];
  for (const [stem, glb] of glbs) {
    const variants = [...(variantsByStem.get(stem) || [])].sort(variantSort);
    result.push({ stem, glb, variants });
  }
  result.sort((a, b) => a.stem.localeCompare(b.stem));
  return result;
}

/** Sort variants with the reserved default ('model') first, then alpha. */
function variantSort(a, b) {
  if (a === DEFAULT_VARIANT) return -1;
  if (b === DEFAULT_VARIANT) return 1;
  return a.localeCompare(b);
}

// ── variant-name validation ─────────────────────────────────────────────

/**
 * Validate a proposed "Save as new variant" name against the reserved
 * default and a set of existing variant names. Returns a result object
 * describing what the caller should do:
 *
 *   { ok: true, variant }                       → safe to write
 *   { ok: false, reason: 'empty' }              → reject (empty/whitespace)
 *   { ok: false, reason: 'reserved' }           → reject (use plain Save)
 *   { ok: true, variant, requiresConfirm: true } → exists; confirm overwrite
 *
 * `existingVariants` may be an array or a Set of variant name strings.
 */
export function validateVariantName(name, existingVariants = []) {
  const trimmed = typeof name === 'string' ? name.trim() : '';
  if (!trimmed) {
    return { ok: false, reason: 'empty' };
  }
  if (trimmed === DEFAULT_VARIANT) {
    return { ok: false, reason: 'reserved' };
  }
  const existing = existingVariants instanceof Set
    ? existingVariants
    : new Set(existingVariants || []);
  if (existing.has(trimmed)) {
    return { ok: true, variant: trimmed, requiresConfirm: true };
  }
  return { ok: true, variant: trimmed };
}

/**
 * The complete rig-sidecar rule set for a serialized sidecar (issue #758).
 *
 * Both write paths call THIS — Models Mode's own Save button
 * (`models-mode-view.js` `writeRig`) and the global Save All route
 * (`save-flow.js`, mode `Models`) — so a sidecar one path refuses can never be
 * written through the other.
 *
 * Combines the object-level marker-name check (`validateRigMarkerNames`, which
 * needs the parsed keys) with the raw-text scan (`validateRigSidecarToml`,
 * which catches duplicate/empty `[markers.<name>]` headers that parsing
 * destroys).
 *
 * @param {string} tomlText  Serialized sidecar.
 * @returns {Array<{path, severity, category, message}>}
 */
export function validateRigSidecarText(tomlText) {
  let rig = null;
  try {
    rig = parseRigToml(tomlText);
  } catch {
    // Unparseable text has no marker keys to name-check; the text scan below
    // still reports the located problems it can see.
    rig = null;
  }
  return [...validateRigMarkerNames(rig), ...validateRigSidecarToml(tomlText)];
}

/**
 * Keep a cross-file `RigIndex` in step with rig-sidecar writes (issue #758).
 *
 * The index is seeded once per project root, but entity saves validate marker
 * references against it synchronously. Without this subscription a marker an
 * author adds in Models Mode is invisible until the editor reloads, and the
 * entity save that references it is refused — a false positive blocking a
 * legitimate save.
 *
 * Both write paths fire `fireModelSaved(path, tomlText)` with the exact text
 * they wrote, so the index is refreshed from memory with no async window.
 *
 * @param {import('./marker-validate.js').RigIndex} rigIndex
 * @param {{ onModelSaved: (cb: (path: string, tomlText: string) => void) => object }} invalidationBus
 * @returns {object|null} the subscription handle, or null if not wired.
 */
export function wireRigIndexToSaves(rigIndex, invalidationBus) {
  if (!rigIndex || typeof invalidationBus?.onModelSaved !== 'function') return null;
  return invalidationBus.onModelSaved((path, tomlText) => {
    if (typeof path !== 'string' || typeof tomlText !== 'string') return;
    try {
      rigIndex.set(path, parseRigToml(tomlText));
    } catch {
      // A just-written sidecar that no longer parses: drop the entry so entity
      // marker checks are SKIPPED rather than judged against a stale rig.
      rigIndex.delete(path);
    }
  });
}
