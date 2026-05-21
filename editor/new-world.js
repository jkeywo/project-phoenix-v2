import { parseWorldToml, stringifyWorldToml } from './world-toml.js';

export function createNewWorldContent() {
  return '[global]\nseed = 42\n\n[anchors]\n';
}

export function getDefaultNewWorldPath() {
  return 'assets/worlds/new_world.toml';
}

export function validateNewWorldPath(path, existingFiles) {
  if (!path.startsWith('assets/worlds/')) {
    return { ok: false, error: 'Path must be under assets/worlds/' };
  }
  if (existingFiles.includes(path)) {
    return { ok: false, error: 'A file already exists at this path' };
  }
  return { ok: true };
}

export function prepareNewWorld(path) {
  const content = createNewWorldContent();
  let parsedContent;
  try {
    parsedContent = parseWorldToml(content);
    const reStrung = stringifyWorldToml(parsedContent);
    const reParsed = parseWorldToml(reStrung);
    if (!reParsed || typeof reParsed.global?.seed !== 'number') {
      return { ok: false, error: 'Round-trip validation failed' };
    }
  } catch (e) {
    return { ok: false, error: e.message };
  }
  return { ok: true, content, parsedContent };
}
