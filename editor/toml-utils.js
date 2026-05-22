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

export function getSpawnsFromAllLayers(layers) {
  const spawns = [];
  for (const layer of layers) {
    const layerSpawns = getSpawns(layer);
    for (const spawn of layerSpawns) {
      spawns.push({ ...spawn, _layer: layer });
    }
  }
  return spawns;
}

export function getEntityPath(spawn) {
  return spawn.template_path;
}

export function getSpawnName(spawn) {
  return spawn.name || spawn.id || 'unnamed';
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