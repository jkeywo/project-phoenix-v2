import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from './workshop-document.js';
import { modelDocuments } from './workshop-models.js';

const NAME = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/;
const VARIANT = /^[A-Za-z0-9_-]{1,64}$/;
const newlineOf = source => source.includes('\r\n') ? '\r\n' : '\n';
const quote = value => JSON.stringify(String(value));
const keySource = value => /^[A-Za-z0-9_-]+$/.test(value) ? value : quote(value);

function headers(source) {
  const matches = [...source.matchAll(/^[ \t]*(\[\[|\[)([^\]\r\n]+)(\]\]|\])[ \t]*(?:#.*)?(?:\r?\n|$)/gm)];
  return matches.filter(match => (match[1] === '[[') === (match[3] === ']]')).map(match => ({
    kind: match[1] === '[[' ? 'array' : 'table', name: match[2].trim(), start: match.index,
    body: match.index + match[0].length, headerEnd: match.index + match[0].length,
  }));
}

function arrayBlocks(source, name) {
  const all = headers(source), out = [];
  for (let index = 0; index < all.length; index += 1) {
    const item = all[index];
    if (item.kind !== 'array' || item.name !== name) continue;
    const next = all.slice(index + 1).find(candidate => candidate.name === name || !candidate.name.startsWith(`${name}.`));
    out.push({ ...item, rootEnd: all.slice(index + 1).find(candidate => candidate.start < (next?.start ?? source.length)
      && candidate.name.startsWith(`${name}.`))?.start ?? (next?.start ?? source.length), end: next?.start ?? source.length });
  }
  return out;
}

function markerHeader(header) {
  if (!header.startsWith('markers.')) return null;
  try {
    const sentinel = '__workshop_marker_header';
    const value = parse(`[${header}]\n${sentinel} = true\n`).markers;
    const keys = value && Object.keys(value);
    if (keys?.length !== 1) return null;
    const name = keys[0], marker = value[name];
    return { name, exact: marker?.[sentinel] === true };
  } catch { return null; }
}

function markerRootEnd(header) {
  let index = 'markers.'.length;
  const quoteMark = header[index];
  if (quoteMark === '"' || quoteMark === "'") {
    index += 1;
    let escaped = false;
    for (; index < header.length; index += 1) {
      const char = header[index];
      if (escaped) escaped = false;
      else if (char === '\\' && quoteMark === '"') escaped = true;
      else if (char === quoteMark) return index + 1;
    }
  }
  while (index < header.length && header[index] !== '.') index += 1;
  return index;
}

function markerBlocks(source) {
  const all = headers(source), out = [];
  for (let index = 0; index < all.length; index += 1) {
    const marker = all[index].kind === 'table' ? markerHeader(all[index].name) : null;
    if (!marker?.exact) continue;
    const following = all.slice(index + 1);
    const rootEnd = following[0]?.start ?? source.length;
    const boundary = following.find(candidate => {
      const child = markerHeader(candidate.name);
      return child?.name !== marker.name || child.exact;
    });
    out.push({ ...all[index], name: marker.name, rootEnd, end: boundary?.start ?? source.length });
  }
  return out;
}

function valueEnd(source, start, limit) {
  let index = start, quoteMark = null, escaped = false, comment = false, square = 0, curly = 0;
  for (; index < limit; index += 1) {
    const char = source[index];
    if (comment) {
      if (char === '\r' || char === '\n') comment = false;
    } else if (quoteMark) {
      if (escaped) escaped = false;
      else if (char === '\\' && quoteMark === '"') escaped = true;
      else if (char === quoteMark) quoteMark = null;
    } else if (char === '"' || char === "'") quoteMark = char;
    else if (char === '[') square += 1;
    else if (char === ']') square -= 1;
    else if (char === '{') curly += 1;
    else if (char === '}') curly -= 1;
    else if (char === '#') {
      if (!square && !curly) break;
      comment = true;
    } else if ((char === '\r' || char === '\n') && !square && !curly) break;
  }
  while (index > start && /\s/.test(source[index - 1])) index -= 1;
  return index;
}

function assignment(source, span, key) {
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = new RegExp(`^[ \\t]*${escaped}[ \\t]*=[ \\t]*`, 'm').exec(source.slice(span.body, span.rootEnd ?? span.end));
  if (!match) return null;
  const lineStart = span.body + match.index, start = lineStart + match[0].length;
  let lineEnd = source.indexOf('\n', valueEnd(source, start, span.rootEnd ?? span.end));
  lineEnd = lineEnd < 0 || lineEnd >= (span.rootEnd ?? span.end) ? (span.rootEnd ?? span.end) : lineEnd + 1;
  return { lineStart, start, end: valueEnd(source, start, span.rootEnd ?? span.end), lineEnd };
}

function tomlValue(value) {
  if (typeof value === 'string') return quote(value);
  if (typeof value === 'number' && Number.isFinite(value)) return String(value);
  if (Array.isArray(value) && value.length === 3 && value.every(item => typeof item === 'number' && Number.isFinite(item))) {
    return `[${value.join(', ')}]`;
  }
  throw new Error('model-structure-value');
}

function sameField(current, next) {
  if (current == null && (next == null || next === '')) return true;
  return JSON.stringify(current) === JSON.stringify(next);
}

function setField(source, span, key, value) {
  const found = assignment(source, span, key);
  if (value == null || value === '') {
    return found ? source.slice(0, found.lineStart) + source.slice(found.lineEnd) : source;
  }
  if (found) return source.slice(0, found.start) + tomlValue(value) + source.slice(found.end);
  const nl = newlineOf(source);
  return source.slice(0, span.rootEnd ?? span.end) + `${key} = ${tomlValue(value)}${nl}` + source.slice(span.rootEnd ?? span.end);
}

function append(source, value) {
  const nl = newlineOf(source), text = stringify(value).trim().replace(/\n/g, nl);
  const gap = !source.length || source.endsWith(`${nl}${nl}`) ? '' : source.endsWith(nl) ? nl : `${nl}${nl}`;
  return source + gap + text + nl;
}

function insertArray(source, name, value, index) {
  const spans = arrayBlocks(source, name);
  if (index == null || index === spans.length) return append(source, { [name]: [value] });
  if (!spans[index]) throw new Error('model-structure-order');
  const nl = newlineOf(source), fragment = stringify({ [name]: [value] }).trim().replace(/\n/g, nl) + nl + nl;
  return source.slice(0, spans[index].start) + fragment + source.slice(spans[index].start);
}

function removeSpan(source, span) { return source.slice(0, span.start) + source.slice(span.end); }

function moveSpan(source, spans, index, direction) {
  const target = index + direction;
  if (!spans[index] || !spans[target]) throw new Error('model-structure-order');
  const firstIndex = Math.min(index, target), secondIndex = Math.max(index, target);
  const first = spans[firstIndex], second = spans[secondIndex];
  return source.slice(0, first.start) + source.slice(second.start, second.end)
    + source.slice(first.end, second.start) + source.slice(first.start, first.end) + source.slice(second.end);
}

function renameMarker(source, span, oldName, newName) {
  const owned = headers(source).filter(header => header.start >= span.start && header.start < span.end
    && markerHeader(header.name)?.name === oldName);
  let result = source;
  for (const header of owned.reverse()) {
    const line = result.slice(header.start, header.body);
    const renamed = `markers.${keySource(newName)}${header.name.slice(markerRootEnd(header.name))}`;
    result = result.slice(0, header.start) + line.replace(header.name, renamed) + result.slice(header.body);
  }
  return result;
}

function lineOf(source, token) {
  const offset = Math.max(0, source.indexOf(token));
  return source.slice(0, offset).split(/\r?\n/).length;
}
const lineAt = (source, offset) => source.slice(0, Math.max(0, offset)).split(/\r?\n/).length;

function refusal(path, source, token, message) {
  const error = new Error(message);
  error.report = { accepted: false, findings: [{ severity: 'error', category: 'model-structure', message,
    file: path, line: lineOf(source, token) }] };
  return error;
}

function vector(value) {
  return Array.isArray(value) && value.length === 3 && value.every(item => Number.isFinite(item));
}

function inventoryPaths(draft, dependencies) {
  const paths = new Set(draft.paths());
  for (const path of Object.keys(dependencies?.base_files || {})) paths.add(path);
  for (const path of Object.keys(dependencies?.base_asset_manifest || {})) paths.add(path);
  for (const pack of dependencies?.packs || []) {
    for (const path of Object.keys(pack.files || {})) paths.add(path);
    for (const path of Object.keys(pack.assets || {})) paths.add(path);
  }
  return paths;
}

export function validateModelStructure(path, source, draft, dependencies = {}, { allowMissingCapturedOutput = false } = {}) {
  let rig;
  try { rig = parse(source); } catch (error) { throw refusal(path, source, '', error.message); }
  for (const [name, marker] of Object.entries(rig.markers || {})) {
    if (!NAME.test(name) || !vector(marker?.position) || !vector(marker?.direction)) {
      throw refusal(path, source, `[markers.${keySource(name)}]`, `Marker ${name} has an invalid runtime transform.`);
    }
    const length = Math.hypot(...marker.direction);
    if (Math.abs(length - 1) > 0.001) throw refusal(path, source, `[markers.${keySource(name)}]`, `Marker ${name} direction must be a unit vector.`);
  }
  for (const [index, point] of (rig.target_points || []).entries()) {
    if (!vector(point?.position)) throw refusal(path, source, '[[target_points]]', `Target point ${index + 1} has an invalid runtime position.`);
  }
  const paths = inventoryPaths(draft, dependencies), bounded = [];
  for (const [index, level] of (rig.lod || []).entries()) {
    const renderers = ['model', 'billboard', 'shape'].filter(key => level[key]);
    if (renderers.length !== 1) throw refusal(path, source, '[[lod]]', `LOD ${index + 1} must select exactly one model, billboard, or shape.`);
    if (level.generate && !level.model) throw refusal(path, source, '[lod.generate]', `LOD ${index + 1} generation requires a model renderer.`);
    if (level.max_distance != null) {
      if (!(Number.isFinite(level.max_distance) && level.max_distance > 0)) throw refusal(path, source, 'max_distance', `LOD ${index + 1} max distance must be positive.`);
      bounded.push(level.max_distance);
    } else if (index !== rig.lod.length - 1) throw refusal(path, source, '[[lod]]', `Only the final LOD may be unbounded.`);
    if (bounded.length > 1 && bounded.at(-1) <= bounded.at(-2)) throw refusal(path, source, 'max_distance', 'LOD distances must increase from near to far.');
    if (level.model && (!level.model.startsWith('assets/models/') || !level.model.endsWith('.glb'))) {
      throw refusal(path, source, String(level.model), `LOD ${index + 1} model must name a runtime GLB asset.`);
    }
    if (level.model && !paths.has(level.model)) throw refusal(path, source, String(level.model), `LOD ${index + 1} names a model asset outside the captured source: ${level.model}`);
    if (level.billboard && (!level.billboard.startsWith('assets/models/') || !level.billboard.endsWith('.png'))) {
      throw refusal(path, source, String(level.billboard), `LOD ${index + 1} billboard must name a runtime PNG asset.`);
    }
    if (level.billboard && !paths.has(level.billboard) && !(allowMissingCapturedOutput && level.capture)) {
      throw refusal(path, source, String(level.billboard), `LOD ${index + 1} names a billboard outside the captured source: ${level.billboard}`);
    }
    if (level.capture) {
      const capture = level.capture;
      if (!level.billboard || typeof capture.source !== 'string' || !capture.source.startsWith('assets/models/')
          || !capture.source.endsWith('.glb') || !paths.has(capture.source)
          || !Number.isInteger(capture.yaw_views) || capture.yaw_views <= 0
          || !Number.isInteger(capture.resolution) || capture.resolution <= 0 || !Number.isFinite(capture.pitch)) {
        throw refusal(path, source, '[lod.capture]', `LOD ${index + 1} has invalid or uncaptured billboard capture metadata.`);
      }
    }
    if (level.variant != null) {
      if (!VARIANT.test(level.variant) || !level.model) throw refusal(path, source, String(level.variant), `LOD ${index + 1} has an invalid model variant reference.`);
      const sidecar = `${level.model.slice(0, -4)}.${level.variant}.toml`;
      if (level.tier_rig !== 'identity' && !paths.has(sidecar)) throw refusal(path, source, String(level.variant), `LOD ${index + 1} variant is not owned by a captured sidecar: ${sidecar}`);
    }
  }
  return rig;
}

export function inspectModelStructure(draft, model, path, dependencies = {}) {
  const entry = modelDocuments(draft.paths()).find(candidate => candidate.model === model);
  if (!entry?.variants.some(variant => variant.path === path)) throw new Error('model-structure-stale');
  const source = draft.read(path), rig = validateModelStructure(path, source, draft, dependencies,
    { allowMissingCapturedOutput: true });
  const markerSpans = markerBlocks(source), targetSpans = arrayBlocks(source, 'target_points'), lodSpans = arrayBlocks(source, 'lod');
  return {
    path, source, variants: entry.variants.map(variant => ({ ...variant, owner: variant.path })),
    markers: markerSpans.map(span => ({ name: span.name, ...structuredClone(rig.markers[span.name]), owner: path,
      line: lineAt(source, span.start) })),
    target_points: (rig.target_points || []).map((point, index) => ({ ...structuredClone(point), owner: path,
      line: lineAt(source, targetSpans[index]?.start) })),
    lod: (rig.lod || []).map((level, index) => ({ ...structuredClone(level), owner: path,
      line: lineAt(source, lodSpans[index]?.start) })),
  };
}

export function prepareModelStructureOperation(draft, dependencies, operation) {
  if (operation.type === 'variant-add') {
    if (!modelDocuments(draft.paths()).some(entry => entry.model === operation.model)
      || !VARIANT.test(operation.name)) throw new Error('workshop.models.invalid_variant');
    const newPath = `${operation.model.slice(0, -4)}.${operation.name}.toml`;
    if (draft.paths().includes(newPath)) throw new Error('workshop.models.variant_exists');
    const source = operation.path ? draft.read(operation.path) : '';
    if (operation.clone && typeof source !== 'string') throw new Error('model-structure-stale');
    return { changes: [{ path: newPath, before: null, after: operation.clone ? source : '' }], selected: newPath };
  }
  const before = draft.read(operation.path);
  if (typeof before !== 'string') throw new Error('model-structure-stale');
  let after = before;
  if (operation.type === 'marker-add') after = append(after, { markers: { [operation.name]: {
    position: operation.position, direction: operation.direction } } });
  else if (operation.type === 'marker-set') {
    const current = parse(before).markers?.[operation.name];
    let spans = markerBlocks(after), span = spans.find(item => item.name === operation.name);
    if (!span) throw refusal(operation.path, before, operation.name, `Marker ${operation.name} is no longer owned by this source.`);
    if (!sameField(current?.position, operation.position)) after = setField(after, span, 'position', operation.position);
    if (!sameField(current?.direction, operation.direction)) {
      spans = markerBlocks(after); span = spans.find(item => item.name === operation.name);
      after = setField(after, span, 'direction', operation.direction);
    }
    if (operation.newName !== operation.name) {
      spans = markerBlocks(after); span = spans.find(item => item.name === operation.name);
      after = renameMarker(after, span, operation.name, operation.newName);
    }
  } else if (operation.type === 'marker-remove') {
    const span = markerBlocks(after).find(item => item.name === operation.name); if (!span) throw new Error('model-structure-stale'); after = removeSpan(after, span);
  } else if (operation.type === 'marker-move') {
    const spans = markerBlocks(after), index = spans.findIndex(item => item.name === operation.name); after = moveSpan(after, spans, index, operation.direction);
  } else if (operation.type === 'target-add') after = append(after, { target_points: [{ position: operation.position }] });
  else if (operation.type === 'target-set') {
    const current = parse(before).target_points?.[operation.index];
    const span = arrayBlocks(after, 'target_points')[operation.index]; if (!span) throw new Error('model-structure-stale');
    if (!sameField(current?.position, operation.position)) after = setField(after, span, 'position', operation.position);
  } else if (operation.type === 'target-remove') {
    const span = arrayBlocks(after, 'target_points')[operation.index]; if (!span) throw new Error('model-structure-stale'); after = removeSpan(after, span);
  } else if (operation.type === 'target-move') after = moveSpan(after, arrayBlocks(after, 'target_points'), operation.index, operation.direction);
  else if (operation.type === 'lod-add') after = insertArray(after, 'lod', operation.level, operation.index);
  else if (operation.type === 'lod-set') {
    const keys = ['max_distance', 'model', 'variant', 'billboard', 'shape'];
    const current = parse(before).lod?.[operation.index];
    for (const key of keys) {
      if (sameField(current?.[key], operation.level[key])) continue;
      const span = arrayBlocks(after, 'lod')[operation.index];
      if (!span) throw new Error('model-structure-stale');
      after = setField(after, span, key, operation.level[key]);
    }
  } else if (operation.type === 'lod-remove') {
    const span = arrayBlocks(after, 'lod')[operation.index]; if (!span) throw new Error('model-structure-stale'); after = removeSpan(after, span);
  } else if (operation.type === 'lod-move') after = moveSpan(after, arrayBlocks(after, 'lod'), operation.index, operation.direction);
  else if (operation.type === 'variant-rename') {
    if (!VARIANT.test(operation.name)) throw new Error('workshop.models.invalid_variant');
    const newPath = `${operation.model.slice(0, -4)}.${operation.name}.toml`;
    if (draft.paths().includes(newPath)) throw new Error('workshop.models.variant_exists');
    return { changes: [{ path: operation.path, before, after: null },
      { path: newPath, before: null, after: before }], selected: newPath };
  } else if (operation.type === 'variant-remove') {
    return { changes: [{ path: operation.path, before, after: null }], selected: null };
  } else throw new Error('model-structure-operation');
  validateModelStructure(operation.path, after, draft, dependencies);
  return { changes: [{ path: operation.path, before, after }], selected: operation.path };
}

export async function applyModelStructureOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const revision = draft.sourceRevision, effective = dependencies || await runtime.dependencies();
  const prepared = prepareModelStructureOperation(draft, effective, operation);
  const candidate = provider?.restoreDocument ? provider.restoreDocument(draft.snapshot()) : WorkshopDocument.restore(draft.snapshot());
  candidate.apply(prepared.changes);
  for (const entry of modelDocuments(candidate.paths())) for (const item of entry.variants) {
    validateModelStructure(item.path, candidate.read(item.path), candidate, effective);
  }
  const report = await runtime.validate(candidate.kind === 'mod' && !provider?.save ? candidate.archive() : null, candidate);
  if (!report?.accepted) { const error = new Error('runtime-validation-refused'); error.report = report; throw error; }
  if (!current() || draft.sourceRevision !== revision || prepared.changes.some(change => draft.read(change.path) !== (change.before ?? undefined))) {
    throw new Error('model-structure-stale');
  }
  draft.apply(prepared.changes);
  return { changes: prepared.changes, selected: prepared.selected, report };
}
