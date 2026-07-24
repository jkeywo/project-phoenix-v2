/**
 * marker-validate.js — PURE model-marker contract validation (issue #758).
 *
 * The JS twin of `src/entities/marker_validate.rs`. Entity TOMLs name rig
 * markers (`marker = "phasers_fore"`, `markers = ["engine_port", …]`) that must
 * exist in the sidecar the entity's `[mesh]` selects. Every runtime resolution
 * path is a silent fallback — a misspelled marker attaches the beam, exhaust,
 * or camera to the ship's centre and nothing complains — so the editor is the
 * place an author finds out.
 *
 * Two entry points:
 *
 *   validateEntityMarkers(parsed, rig)  → findings for one entity's references
 *   validateRigSidecarToml(text)        → findings for a rig sidecar itself
 *
 * Both emit the standard editor finding shape used by `validation.js` and the
 * badge layer: `{ path, severity: 'error'|'warning', message }`. Errors block a
 * save through `SaveFlow` (issue #757), so an invalid attachment is never
 * written to disk.
 *
 * Kept free of DOM, smol-toml, and file IO so it is unit-testable in plain node.
 * `[[system]] marker` is deliberately NOT validated: it is declared-but-unread
 * in the engine (see `SystemInstanceConfig::marker`), so checking it would
 * invent a contract rather than enforce one.
 */

/** Reserved marker-name prefix for camera viewpoints. Mirrors the engine. */
export const CAMERA_MARKER_PREFIX = 'camera_';

/** Reserved default rig-sidecar variant name (mirrors `DEFAULT_VARIANT`). */
export const DEFAULT_VARIANT = 'model';

/** Marker name the viewscreen falls back to (`CameraView::default()`). */
export const DEFAULT_CAMERA_MARKER = 'camera_fore';

export const CATEGORY_MISSING = 'missing-marker';
export const CATEGORY_DUPLICATE = 'duplicate-marker';
export const CATEGORY_INCOMPATIBLE = 'incompatible-marker';
export const CATEGORY_NO_RIG = 'unresolved-model-rig';
export const CATEGORY_MISSING_CAMERA = 'missing-camera-marker';

/**
 * Whether a marker name is compatible with a role. Only the `camera_`
 * namespace is reserved: cameras must sit inside it, weapons and effects must
 * stay out of it.
 */
export function roleAcceptsMarker(role, markerName) {
  const isCamera = typeof markerName === 'string' && markerName.startsWith(CAMERA_MARKER_PREFIX);
  return role === 'camera' ? isCamera : !isCamera;
}

/**
 * Sidecar path for a model path + variant. Mirrors
 * `src/entities/model_rig.rs::sidecar_path`:
 *   ('assets/models/x.glb', undefined) -> 'assets/models/x.model.toml'
 *   ('assets/models/x.glb', 'large')   -> 'assets/models/x.large.toml'
 */
export function sidecarPathFor(modelPath, variant) {
  if (typeof modelPath !== 'string' || !modelPath) return null;
  const v = typeof variant === 'string' && variant.trim() ? variant.trim() : DEFAULT_VARIANT;
  const stem = modelPath.toLowerCase().endsWith('.glb')
    ? modelPath.slice(0, -'.glb'.length)
    : modelPath;
  return `${stem}.${v}.toml`;
}

/**
 * Collect every authored marker reference in a parsed entity TOML, in a stable
 * order: phaser banks, blaster banks, torpedo tubes, engine exhaust PFX.
 *
 * Each ref is `{ role, owner, name, path }`, where `path` is the indexed
 * validation path the badge layer decorates.
 */
export function collectMarkerRefs(parsed) {
  const refs = [];
  if (!parsed || typeof parsed !== 'object') return refs;

  const banks = (list, section, label) => {
    if (!Array.isArray(list)) return;
    for (let i = 0; i < list.length; i++) {
      const entry = list[i];
      const name = entry && typeof entry.marker === 'string' ? entry.marker.trim() : '';
      if (!name) continue;
      refs.push({
        role: 'weapon',
        owner: `${label} "${entry.id ?? i}"`,
        name,
        path: `${section}[${i}].marker`,
      });
    }
  };

  banks(parsed.weapons_console?.phaser_banks, 'weapons_console.phaser_banks', 'Phaser bank');
  banks(parsed.weapons_console?.blaster_banks, 'weapons_console.blaster_banks', 'Blaster bank');
  banks(parsed.torpedoes?.tubes, 'torpedoes.tubes', 'Torpedo tube');

  // Per-barrel markers (issue #765): each authored barrel-marker name is its
  // own reference, so a missing/incompatible barrel marker is rejected just
  // like the bank's single `marker`.
  const blasterBanks = parsed.weapons_console?.blaster_banks;
  if (Array.isArray(blasterBanks)) {
    for (let i = 0; i < blasterBanks.length; i++) {
      const entry = blasterBanks[i];
      const barrels = entry && Array.isArray(entry.barrels) ? entry.barrels : null;
      if (!barrels) continue;
      for (let b = 0; b < barrels.length; b++) {
        const name = typeof barrels[b] === 'string' ? barrels[b].trim() : '';
        if (!name) continue;
        refs.push({
          role: 'weapon',
          owner: `Blaster bank "${entry.id ?? i}" barrel ${b}`,
          name,
          path: `weapons_console.blaster_banks[${i}].barrels[${b}]`,
        });
      }
    }
  }

  // Per-barrel torpedo markers (issue #766): each authored barrel-marker name
  // is its own reference, mirroring the blaster loop above.
  const torpedoTubes = parsed.torpedoes?.tubes;
  if (Array.isArray(torpedoTubes)) {
    for (let i = 0; i < torpedoTubes.length; i++) {
      const entry = torpedoTubes[i];
      const barrels = entry && Array.isArray(entry.barrels) ? entry.barrels : null;
      if (!barrels) continue;
      for (let b = 0; b < barrels.length; b++) {
        const name = typeof barrels[b] === 'string' ? barrels[b].trim() : '';
        if (!name) continue;
        refs.push({
          role: 'weapon',
          owner: `Torpedo tube "${entry.id ?? i}" barrel ${b}`,
          name,
          path: `torpedoes.tubes[${i}].barrels[${b}]`,
        });
      }
    }
  }

  const pfxMarkers = parsed.helm_console?.engine_pfx?.markers;
  if (Array.isArray(pfxMarkers)) {
    for (let i = 0; i < pfxMarkers.length; i++) {
      const name = typeof pfxMarkers[i] === 'string' ? pfxMarkers[i].trim() : '';
      if (!name) continue;
      refs.push({
        role: 'effect',
        owner: 'Engine exhaust PFX',
        name,
        path: `helm_console.engine_pfx.markers[${i}]`,
      });
    }
  }

  return refs;
}

/** The set of marker names a parsed rig declares. `rig.markers` is an object. */
function markerNames(rig) {
  if (!rig || typeof rig !== 'object' || !rig.markers || typeof rig.markers !== 'object') {
    return null;
  }
  return Object.keys(rig.markers);
}

function checkRef(ref, names) {
  if (names === null) {
    return {
      path: ref.path,
      severity: 'error',
      category: CATEGORY_NO_RIG,
      message: `${ref.owner} references marker "${ref.name}" but no model rig could be resolved for this entity`,
    };
  }
  if (!names.includes(ref.name)) {
    const known = [...names].sort().join(', ');
    return {
      path: ref.path,
      severity: 'error',
      category: CATEGORY_MISSING,
      message: `${ref.owner} references marker "${ref.name}" which the model rig does not declare (declared: [${known}])`,
    };
  }
  if (!roleAcceptsMarker(ref.role, ref.name)) {
    return {
      path: ref.path,
      severity: 'error',
      category: CATEGORY_INCOMPATIBLE,
      message: `${ref.owner} references marker "${ref.name}": the "${CAMERA_MARKER_PREFIX}" prefix is reserved for camera viewpoints`,
    };
  }
  return null;
}

/**
 * Validate one camera view name against a rig. Used for the default viewscreen
 * marker and by any caller resolving an explicit view.
 */
export function validateCameraView(markerName, rig, path = 'mesh.model') {
  const finding = checkRef(
    { role: 'camera', owner: 'Camera view', name: markerName, path },
    markerNames(rig),
  );
  return finding ? [finding] : [];
}

/**
 * Validate every marker reference in a parsed entity TOML against `rig`.
 *
 * @param {object} parsed  Parsed entity TOML.
 * @param {object|null} rig  Parsed rig sidecar (`{ markers: { name: {...} } }`),
 *   or null/undefined when the entity selects no model or the sidecar could not
 *   be resolved. An entity with no marker references is always clean.
 * @returns {Array<{path, severity, category, message}>}
 */
export function validateEntityMarkers(parsed, rig) {
  const refs = collectMarkerRefs(parsed);
  const names = markerNames(rig);
  const findings = [];
  for (const ref of refs) {
    const f = checkRef(ref, names);
    if (f) findings.push(f);
  }

  // A hull a player can fly needs the default viewscreen camera, or the view
  // silently snaps to the ship's origin.
  if (parsed?.captain_console && names !== null && !names.includes(DEFAULT_CAMERA_MARKER)) {
    findings.push({
      path: 'mesh.model',
      severity: 'warning',
      category: CATEGORY_MISSING_CAMERA,
      message: `Hull has a captain console but its model rig declares no "${DEFAULT_CAMERA_MARKER}" marker; the viewscreen falls back to the ship's origin`,
    });
  }

  return findings;
}

/**
 * Validate a rig sidecar's own TOML text. Scans for a `[markers.<name>]` table
 * declared twice — both the TOML parser (hard error) and the parsed object
 * (silent last-wins) destroy the evidence, so this runs on raw text and reports
 * a located finding instead.
 *
 * Also flags a `[markers."<name>"]` whose name is empty.
 */
export function validateRigSidecarToml(text) {
  if (typeof text !== 'string') return [];
  const seen = new Set();
  const findings = [];
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const trimmed = lines[i].trim();
    if (!trimmed.startsWith('[markers.') || !trimmed.endsWith(']')) continue;
    const name = trimmed.slice('[markers.'.length, -1).trim().replace(/^"|"$/g, '');
    if (!name) {
      findings.push({
        path: `markers[line ${i + 1}]`,
        severity: 'error',
        category: CATEGORY_MISSING,
        message: `Line ${i + 1}: marker name must not be empty`,
      });
      continue;
    }
    if (seen.has(name)) {
      findings.push({
        path: `markers.${name}`,
        severity: 'error',
        category: CATEGORY_DUPLICATE,
        message: `Line ${i + 1}: model rig declares marker "${name}" more than once`,
      });
    }
    seen.add(name);
  }
  return findings;
}

/**
 * A marker name must be a TOML *bare key* so it round-trips as
 * `[markers.<name>]`. Anything outside this set (spaces, dots, quotes) either
 * makes the sidecar unparseable — in which case the engine degrades to an
 * identity rig and EVERY marker in the file is lost — or silently changes the
 * key an entity has to reference.
 */
export const MARKER_NAME_PATTERN = /^[A-Za-z0-9_-]+$/;

/** Whether `name` is usable as a `[markers.<name>]` key. */
export function isValidMarkerName(name) {
  return typeof name === 'string' && MARKER_NAME_PATTERN.test(name);
}

/**
 * Validate the marker names of a rig object (`{ markers: { name: {...} } }`).
 * Complements `validateRigSidecarToml`, which works on raw text.
 */
export function validateRigMarkerNames(rig) {
  const names = markerNames(rig);
  if (names === null) return [];
  const findings = [];
  for (const name of names) {
    if (!isValidMarkerName(name)) {
      findings.push({
        path: `markers.${name}`,
        severity: 'error',
        category: CATEGORY_INCOMPATIBLE,
        message: `Marker name "${name}" is not a valid rig key — use letters, digits, "_" or "-" only`,
      });
    }
  }
  return findings;
}

/**
 * A sync lookup from sidecar path → parsed rig, so the (synchronous)
 * validators can do a cross-file check. Populate it from an async file read at
 * mount time; `get` never does IO.
 */
export class RigIndex {
  constructor() {
    this._rigs = new Map();
  }

  /** Store a parsed rig (`{ markers: {...} }`) for a sidecar path. */
  set(sidecarPath, rig) {
    this._rigs.set(sidecarPath, rig);
    return this;
  }

  /**
   * Forget a sidecar path. Used when a freshly written sidecar no longer
   * parses: "unknown" (checks skipped) is a safer state than a stale rig that
   * would report the author's new markers as missing.
   */
  delete(sidecarPath) {
    this._rigs.delete(sidecarPath);
    return this;
  }

  /**
   * Forget every indexed sidecar. Used when the project root changes: rigs from
   * the previous root share relative paths with the new one, so keeping them
   * would validate the new root's entities against the old root's markers.
   */
  clear() {
    this._rigs.clear();
    return this;
  }

  /** Whether a sidecar path has been indexed at all. */
  has(sidecarPath) {
    return this._rigs.has(sidecarPath);
  }

  /**
   * The rig for a sidecar path, or `undefined` when the path was never
   * indexed. `undefined` means "unknown" — callers must skip validation rather
   * than report every marker as missing.
   */
  get(sidecarPath) {
    return this._rigs.get(sidecarPath);
  }

  /**
   * Resolve the rig an entity's `[mesh]` selects. Returns `undefined` when the
   * entity selects no model or the sidecar is not indexed.
   */
  forEntity(parsed) {
    const model = parsed?.mesh?.model;
    if (typeof model !== 'string' || !model) return undefined;
    return this.get(sidecarPathFor(model, parsed?.mesh?.variant));
  }
}
