export async function parseToml(text) {
  if (!window.tomlParse) {
    await import('https://esm.sh/smol-toml@1.3.1');
  }
  return window.tomlParse(text);
}

export function stringifyToml(obj) {
  if (!window.tomlStringify) {
    throw new Error('TOML stringify not loaded');
  }
  return window.tomlStringify(obj);
}

export function getSpawns(layer) {
  if (layer.isMap) {
    return layer.toml.entity || [];
  }
  return [];
}

export function getAnchors(layer) {
  if (!layer.isMap || !layer.toml.anchors) {
    return [];
  }
  return Object.entries(layer.toml.anchors).map(([name, pos]) => ({
    name,
    position: pos
  }));
}

export function getAllAnchors(layers) {
  const anchors = [];
  for (const layer of layers) {
    anchors.push(...getAnchors(layer));
  }
  return anchors;
}

export function getEntityPath(spawn) {
  return spawn.template_path;
}

export function getSpawnName(spawn) {
  return spawn.name || spawn.id || 'unnamed';
}

/**
 * The identifier a runtime `relative_to` resolves against: `name` first, then
 * `id`, and `null` when the spawn authors neither.
 *
 * Deliberately not `getSpawnName`, which falls back to the literal string
 * `'unnamed'` so the tree and the canvas always have something to draw.
 * `'unnamed'` is not an authored identifier, so a `relative_to = "unnamed"`
 * matches no entity — and since issue #969 that no longer costs one misplaced
 * entity, it fails validation and blocks the entire world. Display text and
 * reference ids are different jobs; only this one may reach an `<option value>`.
 *
 * @param {object} spawn
 * @returns {string|null}
 */
export function getSpawnReference(spawn) {
  return spawn.name || spawn.id || null;
}

/**
 * Does `spawn` answer to an already-authored `relative_to` reference?
 *
 * This is the *reading* direction, and it is not the inverse of
 * `getSpawnReference`. `build_named_entity_positions` (`src/world/config.rs`)
 * keys the runtime table by BOTH `id` and `name` since issue #969, so a spawn
 * carrying both answers to either spelling. `getSpawnReference` has to pick one
 * of them to write into a new `<option value>`; asking whether an existing
 * reference resolves must accept both, or every shipped landmark reads back as
 * unresolved — each carries a short `id` (`gas-giant`) plus a strings.csv `name`
 * (`world.entity.gas_giant.name`), and every shipped `relative_to` names the
 * `id`, which is the losing side of `getSpawnReference`'s `name`-first pick.
 *
 * A `null`/`undefined` reference matches nothing: a spawn with neither `id` nor
 * `name` must not compare equal to an absent reference by way of
 * `undefined === undefined`.
 *
 * @param {object} spawn
 * @param {string|null|undefined} reference
 * @returns {boolean}
 */
export function matchesSpawnReference(spawn, reference) {
  if (reference === null || reference === undefined) return false;
  return spawn.name === reference || spawn.id === reference;
}

/**
 * The spawns a `relative_to` on `subject` may legally name, mirroring what
 * `build_named_entity_positions` (`src/world/config.rs`) will actually put in
 * the runtime lookup table. Three rules, each of which the picker used to break:
 *
 * 1. **Same layer only.** The runtime table is built from one `WorldConfig`, so
 *    a reference into another open layer resolves against nothing.
 * 2. **Must have a `name` or an `id`.** Anonymous spawns are unnameable; the
 *    shipped worlds have several, and every one of them used to be offered
 *    under the label `'unnamed'`.
 * 3. **Must not itself be `relative_to`-positioned.** Chains are unsupported by
 *    design and such an entity is excluded from the table.
 *
 * `subject` is also excluded: an entity cannot be positioned relative to
 * itself — setting `relative_to` takes it straight out of the table.
 *
 * @param {object} layer - The layer the subject spawn lives in.
 * @param {object|null} subject - The spawn being edited, excluded from the result.
 * @returns {object[]}
 */
export function getRelativeToCandidates(layer, subject = null) {
  return getSpawns(layer).filter(spawn =>
    spawn !== subject &&
    getSpawnReference(spawn) !== null &&
    !(spawn.transform && spawn.transform.relative_to)
  );
}

export function setSpawnPosition(spawn, x, z, mode = 'absolute', parent = null, offset = null) {
  // Spawn positioning lives under a nested `transform` sub-object.
  if (!spawn.transform) spawn.transform = {};
  const t = spawn.transform;
  if (mode === 'absolute') {
    t.position = [x, 0, z];
    delete t.anchor;
    delete t.relative_to;
    delete t.offset;
  } else if (mode === 'anchor') {
    t.anchor = parent;
    delete t.position;
    delete t.relative_to;
    delete t.offset;
  } else if (mode === 'relative') {
    t.relative_to = parent;
    t.offset = [offset.x, 0, offset.z];
    delete t.position;
    delete t.anchor;
  }
  if (Object.keys(spawn.transform).length === 0) {
    delete spawn.transform;
  }
}

export function getSpawnPosition(spawn, anchors = []) {
  const t = spawn.transform;
  if (!t) return { x: 0, z: 0 };
  if (t.position && Array.isArray(t.position)) {
    return { x: t.position[0], z: t.position[2] };
  }
  if (t.anchor && anchors.length > 0) {
    const anchor = anchors.find(a => a.name === t.anchor);
    if (anchor && Array.isArray(anchor.position)) {
      return { x: anchor.position[0], z: anchor.position[2] };
    }
  }
  return { x: 0, z: 0 };
}

export function getRelativeInfo(spawn) {
  const t = spawn.transform;
  if (t && t.relative_to && t.offset) {
    return {
      parent: t.relative_to,
      offset: { x: t.offset[0], z: t.offset[2] }
    };
  }
  return null;
}

// Rotation is XYZ Euler in radians; mirrors Rust `TransformConfig::rotation`.
// Default `[0,0,0]` is omitted from TOML to keep round-trips byte-clean.
export function setSpawnRotation(spawn, rot) {
  const [x, y, z] = rot;
  const isDefault = x === 0 && y === 0 && z === 0;
  if (isDefault) {
    if (spawn.transform) {
      delete spawn.transform.rotation;
      if (Object.keys(spawn.transform).length === 0) delete spawn.transform;
    }
    return;
  }
  if (!spawn.transform) spawn.transform = {};
  spawn.transform.rotation = [x, y, z];
}

export function getSpawnRotation(spawn) {
  const r = spawn.transform?.rotation;
  if (Array.isArray(r) && r.length === 3) return [r[0], r[1], r[2]];
  return [0, 0, 0];
}

// Scale defaults to `[1,1,1]` (`Vec3::ONE`); that value is omitted from TOML.
export function setSpawnScale(spawn, scl) {
  const [x, y, z] = scl;
  const isDefault = x === 1 && y === 1 && z === 1;
  if (isDefault) {
    if (spawn.transform) {
      delete spawn.transform.scale;
      if (Object.keys(spawn.transform).length === 0) delete spawn.transform;
    }
    return;
  }
  if (!spawn.transform) spawn.transform = {};
  spawn.transform.scale = [x, y, z];
}

export function getSpawnScale(spawn) {
  const s = spawn.transform?.scale;
  if (Array.isArray(s) && s.length === 3) return [s[0], s[1], s[2]];
  return [1, 1, 1];
}