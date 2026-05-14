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

export function inferLayerKind(toml) {
  if (toml.spawn && Array.isArray(toml.spawn)) {
    return 'scenario';
  }
  if (toml.anchors || (toml.entity && Array.isArray(toml.entity))) {
    return 'map';
  }
  return 'unknown';
}

export function getSpawns(layer) {
  if (layer.kind === 'scenario') {
    return layer.toml.spawn || [];
  }
  if (layer.kind === 'map') {
    return layer.toml.entity || [];
  }
  return [];
}

export function getAnchors(layer) {
  if (layer.kind !== 'map' || !layer.toml.anchors) {
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
  return spawn.entity_path || spawn.template_path;
}

export function getSpawnName(spawn) {
  return spawn.name || spawn.id || 'unnamed';
}

export function setSpawnPosition(spawn, x, z, mode = 'absolute', parent = null, offset = null) {
  if (mode === 'absolute') {
    spawn.position = [x, 0, z];
    delete spawn.anchor;
    delete spawn.relative_to;
    delete spawn.offset;
  } else if (mode === 'anchor') {
    spawn.anchor = parent;
    delete spawn.position;
    delete spawn.relative_to;
    delete spawn.offset;
  } else if (mode === 'relative') {
    spawn.relative_to = parent;
    spawn.offset = [offset.x, 0, offset.z];
    delete spawn.position;
    delete spawn.anchor;
  }
}

export function getSpawnPosition(spawn, anchors = []) {
  if (spawn.position && Array.isArray(spawn.position)) {
    return { x: spawn.position[0], z: spawn.position[2] };
  }
  if (spawn.anchor && anchors.length > 0) {
    const anchor = anchors.find(a => a.name === spawn.anchor);
    if (anchor && Array.isArray(anchor.position)) {
      return { x: anchor.position[0], z: anchor.position[2] };
    }
  }
  return { x: 0, z: 0 };
}

export function getRelativeInfo(spawn) {
  if (spawn.relative_to && spawn.offset) {
    return {
      parent: spawn.relative_to,
      offset: { x: spawn.offset[0], z: spawn.offset[2] }
    };
  }
  return null;
}