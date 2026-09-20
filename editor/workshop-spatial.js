import { parse } from 'smol-toml';
import { setExtraWorlds } from './workshop-composition.js';
import { resolveTemplate } from './entity-includes.js';
import { WorkshopDocument } from './workshop-document.js';

const WORLD_PREFIX = 'assets/worlds/';
const worldPath = path => typeof path === 'string' && path.startsWith(WORLD_PREFIX)
  && path.endsWith('.toml') && !path.includes('\\') && !path.split('/').includes('..');
const newlineOf = source => source.includes('\r\n') ? '\r\n' : '\n';
const quote = value => JSON.stringify(value);
const finite3 = value => Array.isArray(value) && value.length === 3
  && value.every(Number.isFinite);
const tomlKey = value => /^[A-Za-z0-9_-]+$/.test(value) ? value : quote(value);

function parsed(source, path) {
  try { return parse(source); }
  catch (error) { throw new Error(`${path}: ${error.message}`); }
}

function valueEnd(source, start) {
  let index = start, quoted = null, escaped = false, square = 0, curly = 0;
  while (index < source.length) {
    const character = source[index];
    if (quoted) {
      if (escaped) escaped = false;
      else if (character === '\\' && quoted === '"') escaped = true;
      else if (character === quoted) quoted = null;
    } else if (character === '"' || character === "'") quoted = character;
    else if (character === '[') square += 1;
    else if (character === ']') square -= 1;
    else if (character === '{') curly += 1;
    else if (character === '}') curly -= 1;
    else if ((character === '#' || character === '\r' || character === '\n') && !square && !curly) break;
    index += 1;
  }
  while (index > start && /\s/.test(source[index - 1])) index -= 1;
  return index;
}

function headers(source) {
  return [...source.matchAll(/^[ \t]*(\[{1,2})([^\]\r\n]+)(\]{1,2})[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, body: match.index + match[0].length,
      name: match[2].trim(), array: match[1] === '[[' && match[3] === ']]' }));
}

function tableSpan(source, name) {
  const all = headers(source), index = all.findIndex(row => !row.array && row.name === name);
  if (index < 0) return null;
  return { ...all[index], end: all[index + 1]?.start ?? source.length };
}

function entitySpans(source) {
  const all = headers(source), starts = all.map((row, index) => ({ row, index }))
    .filter(({ row }) => row.array && row.name === 'entity');
  return starts.map(({ row, index }) => {
    const boundary = all.slice(index + 1).find(candidate => candidate.name === 'entity'
      ? candidate.array : !candidate.name.startsWith('entity.'));
    return { ...row, end: boundary?.start ?? source.length };
  });
}

function assignmentSpan(source, span, key) {
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = new RegExp(`^[ \\t]*${escaped}[ \\t]*=[ \\t]*`, 'm').exec(source.slice(span.body, span.end));
  if (!match) return null;
  const start = span.body + match.index + match[0].length;
  return { start, end: valueEnd(source, start), lineStart: span.body + match.index };
}

function tomlValue(value) {
  if (typeof value === 'string') return quote(value);
  if (typeof value === 'number' && Number.isFinite(value)) return String(value);
  if (typeof value === 'boolean') return String(value);
  if (Array.isArray(value)) return `[${value.map(tomlValue).join(', ')}]`;
  if (value && typeof value === 'object') return `{ ${Object.entries(value).map(([key, child]) =>
    `${tomlKey(key)} = ${tomlValue(child)}`).join(', ')} }`;
  throw new Error('invalid-transform-value');
}

function transformValue(transform = {}) {
  const ordered = ['position', 'anchor', 'relative_to', 'offset', 'rotation', 'scale'];
  const entries = [...ordered, ...Object.keys(transform).filter(key => !ordered.includes(key))]
    .filter((key, index, keys) => keys.indexOf(key) === index && transform[key] !== undefined)
    .map(key => `${tomlKey(key)} = ${tomlValue(transform[key])}`);
  if (!entries.some(row => row.startsWith('position =') || row.startsWith('anchor =') || row.startsWith('relative_to ='))) {
    throw new Error('placement-required');
  }
  return `{ ${entries.join(', ')} }`;
}

function inlineValueEnd(source, start) {
  let index = start, quoted = null, escaped = false, square = 0, curly = 0;
  while (index < source.length) {
    const character = source[index];
    if (quoted) {
      if (escaped) escaped = false;
      else if (character === '\\' && quoted === '"') escaped = true;
      else if (character === quoted) quoted = null;
    } else if (character === '"' || character === "'") quoted = character;
    else if (character === '[') square += 1;
    else if (character === ']') square -= 1;
    else if (character === '{') curly += 1;
    else if (character === '}' && curly) curly -= 1;
    else if ((character === ',' || character === '}') && !square && !curly) break;
    index += 1;
  }
  while (index > start && /\s/.test(source[index - 1])) index -= 1;
  return index;
}

function inlineEntry(source, key) {
  const escaped = tomlKey(key).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = new RegExp(`([,{])([ \\t\\r\\n]*)${escaped}[ \\t]*=[ \\t]*`).exec(source);
  if (!match) return null;
  const valueStart = match.index + match[0].length;
  return { delimiter: match[1], delimiterStart: match.index, keyStart: match.index + 1,
    valueStart, valueEnd: inlineValueEnd(source, valueStart) };
}

function patchInlineTransform(raw, edits, removals) {
  let result = raw;
  for (const key of removals) {
    const found = inlineEntry(result, key);
    if (!found) continue;
    if (found.delimiter === '{') {
      let end = found.valueEnd;
      while (/\s/.test(result[end] || '')) end += 1;
      if (result[end] === ',') end += 1;
      result = result.slice(0, found.keyStart) + result.slice(end);
    } else result = result.slice(0, found.delimiterStart) + result.slice(found.valueEnd);
  }
  for (const [key, value] of Object.entries(edits).filter(([, value]) => value !== undefined)) {
    const found = inlineEntry(result, key);
    if (found) result = result.slice(0, found.valueStart) + tomlValue(value) + result.slice(found.valueEnd);
    else {
      const close = result.lastIndexOf('}');
      if (close < 0) throw new Error('transform-source-unavailable');
      const occupied = result.slice(1, close).trim().length > 0;
      result = result.slice(0, close) + `${occupied ? ', ' : ''}${tomlKey(key)} = ${tomlValue(value)}` + result.slice(close);
    }
  }
  return result;
}

function replaceEntityTransform(source, index, transform, edits = transform, removals = []) {
  const span = entitySpans(source)[index];
  if (!span) throw new Error('stale-entity');
  const assignment = assignmentSpan(source, span, 'transform');
  const replacement = transformValue(transform);
  if (assignment) {
    const raw = source.slice(assignment.start, assignment.end);
    return source.slice(0, assignment.start) + patchInlineTransform(raw, edits, removals) + source.slice(assignment.end);
  }
  const nested = headers(source).find(row => !row.array && row.name === 'entity.transform'
    && row.start >= span.body && row.start < span.end);
  if (nested) {
    const next = headers(source).find(row => row.start > nested.start && row.start <= span.end);
    const table = { ...nested, end: next?.start ?? span.end };
    let block = source.slice(table.body, table.end);
    for (const key of removals) {
      const found = assignmentSpan(block, { body: 0, end: block.length }, tomlKey(key));
      if (!found) continue;
      let end = block.indexOf('\n', found.end); end = end < 0 ? block.length : end + 1;
      block = block.slice(0, found.lineStart) + block.slice(end);
    }
    for (const [key, value] of Object.entries(edits).filter(([, value]) => value !== undefined)) {
      const found = assignmentSpan(block, { body: 0, end: block.length }, tomlKey(key));
      if (found) block = block.slice(0, found.start) + tomlValue(value) + block.slice(found.end);
      else {
        const newline = newlineOf(source);
        block += `${tomlKey(key)} = ${tomlValue(value)}${newline}`;
      }
    }
    return source.slice(0, table.body) + block + source.slice(table.end);
  }
  const newline = newlineOf(source);
  return source.slice(0, span.body) + `transform = ${replacement}${newline}` + source.slice(span.body);
}

function spatialComponent(draft, start) {
  const worlds = new Map(draft.paths().filter(worldPath).map(path => [path, parsed(draft.read(path), path)]));
  const connected = new Set([start]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const [path, world] of worlds) {
      const links = Array.isArray(world.extra_worlds) ? world.extra_worlds : [];
      if (connected.has(path) || links.some(link => connected.has(link))) {
        for (const link of [path, ...links]) if (worlds.has(link) && !connected.has(link)) {
          connected.add(link); changed = true;
        }
      }
    }
  }
  return connected;
}

function anchorsUsed(draft, name, owner) {
  const users = [];
  for (const path of spatialComponent(draft, owner)) {
    const source = draft.read(path);
    if (typeof source !== 'string') continue;
    const world = parsed(source, path);
    for (const [index, entity] of (world.entity || []).entries()) {
      if (entity?.transform?.anchor === name) users.push({ path, index, id: entity.id || entity.name || null });
    }
  }
  return users;
}

function setAnchor(source, name, value, { create = false } = {}) {
  if (!finite3(value) || typeof name !== 'string' || !name) throw new Error('invalid-anchor');
  const span = tableSpan(source, 'anchors'), key = tomlKey(name), replacement = `[${value.join(', ')}]`;
  if (!span) {
    if (!create) throw new Error('stale-anchor');
    const newline = newlineOf(source), gap = !source.length || source.endsWith(newline) ? newline : `${newline}${newline}`;
    return `${source}${gap}[anchors]${newline}${key} = ${replacement}${newline}`;
  }
  const assignment = assignmentSpan(source, span, key);
  if (assignment) {
    if (create) throw new Error('duplicate-anchor');
    return source.slice(0, assignment.start) + replacement + source.slice(assignment.end);
  }
  if (!create) throw new Error('stale-anchor');
  const newline = newlineOf(source), before = source.slice(0, span.end);
  return before + (before.endsWith(newline) ? '' : newline) + `${key} = ${replacement}${newline}` + source.slice(span.end);
}

function removeAnchor(source, name) {
  const span = tableSpan(source, 'anchors');
  if (!span) throw new Error('stale-anchor');
  const assignment = assignmentSpan(source, span, tomlKey(name));
  if (!assignment) throw new Error('stale-anchor');
  let end = source.indexOf('\n', assignment.end);
  end = end < 0 ? source.length : end + 1;
  return source.slice(0, assignment.lineStart) + source.slice(end);
}

function appendEntity(source, operation) {
  if (typeof operation.template_path !== 'string' || !operation.template_path.startsWith('assets/entities/')
    || !operation.template_path.endsWith('.toml') || operation.template_path.includes('\\')
    || operation.template_path.split('/').includes('..')) throw new Error('invalid-template');
  const newline = newlineOf(source), lines = ['[[entity]]', `template_path = ${quote(operation.template_path)}`];
  if (operation.id) lines.push(`id = ${quote(operation.id)}`);
  if (operation.name) lines.push(`name = ${quote(operation.name)}`);
  lines.push(`transform = ${transformValue(operation.transform)}`);
  const gap = !source.length || source.endsWith(`${newline}${newline}`) ? '' : source.endsWith(newline) ? newline : `${newline}${newline}`;
  return source + gap + lines.join(newline) + newline;
}

function removeEntity(source, index) {
  const span = entitySpans(source)[index];
  if (!span) throw new Error('stale-entity');
  return source.slice(0, span.start) + source.slice(span.end);
}

export function spatialInventory(draft, dependencies = { base_files: {}, packs: [] }) {
  const templates = {};
  Object.assign(templates, dependencies.base_files || {});
  for (const pack of dependencies.packs || []) Object.assign(templates, pack.files || {});
  for (const path of draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path))) {
    templates[path] = draft.read(path);
  }
  const regionTemplate = path => {
    if (typeof templates[path] !== 'string') return false;
    const result = resolveTemplate(path, templates, parse);
    return Boolean(result.ok && Array.isArray(result.resolved.value.tags) && result.resolved.value.tags.includes('region'));
  };
  return draft.paths().filter(path => worldPath(path) && !draft.isBinary(path)).sort().map(path => {
    const source = draft.read(path), world = parsed(source, path);
    return { path, anchors: Object.entries(world.anchors || {}).map(([name, position]) => ({ name, position })),
      layers: Array.isArray(world.extra_worlds) ? [...world.extra_worlds] : [],
      entities: (world.entity || []).map((entity, index) => ({ index, id: entity.id || null,
        name: entity.name || null, template_path: entity.template_path, transform: entity.transform || {},
        region: regionTemplate(entity.template_path) })) };
  });
}

export function prepareSpatialOperation(draft, operation) {
  if (!worldPath(operation.path) || typeof draft.read(operation.path) !== 'string') throw new Error('stale-world');
  const source = draft.read(operation.path), world = parsed(source, operation.path);
  let changes;
  if (operation.type === 'anchor-add') changes = [{ path: operation.path, before: source,
    after: setAnchor(source, operation.name, operation.position, { create: true }) }];
  else if (operation.type === 'anchor-move') changes = [{ path: operation.path, before: source,
    after: setAnchor(source, operation.name, operation.position) }];
  else if (operation.type === 'anchor-remove') {
    const users = anchorsUsed(draft, operation.name, operation.path);
    if (users.length) { const error = new Error('anchor-in-use'); error.users = users; throw error; }
    changes = [{ path: operation.path, before: source, after: removeAnchor(source, operation.name) }];
  } else if (operation.type === 'entity-add') changes = [{ path: operation.path, before: source,
    after: appendEntity(source, operation) }];
  else if (operation.type === 'entity-move') {
    const entity = (world.entity || [])[operation.index];
    if (!entity) throw new Error('stale-entity');
    const transform = { ...entity.transform, ...operation.transform };
    const removals = [];
    if (operation.transform.position) {
      transform.anchor = undefined; transform.relative_to = undefined; transform.offset = undefined;
      removals.push('anchor', 'relative_to', 'offset');
    } else if (operation.transform.anchor) {
      transform.position = undefined; transform.relative_to = undefined; transform.offset = undefined;
      removals.push('position', 'relative_to', 'offset');
    }
    changes = [{ path: operation.path, before: source,
      after: replaceEntityTransform(source, operation.index, transform, operation.transform, removals) }];
  } else if (operation.type === 'entity-remove') changes = [{ path: operation.path, before: source,
    after: removeEntity(source, operation.index) }];
  else if (operation.type === 'layer-add') {
    if (!worldPath(operation.layer) || draft.read(operation.layer) !== undefined) throw new Error('invalid-layer');
    const layers = Array.isArray(world.extra_worlds) ? [...world.extra_worlds] : [];
    if (layers.includes(operation.layer)) throw new Error('duplicate-layer');
    changes = [{ path: operation.path, before: source, after: setExtraWorlds(source, [...layers, operation.layer]) },
      { path: operation.layer, before: null, after: `# Workshop spatial layer${newlineOf(source)}[anchors]${newlineOf(source)}` }];
  } else if (operation.type === 'layer-remove') {
    const layers = Array.isArray(world.extra_worlds) ? [...world.extra_worlds] : [], index = layers.indexOf(operation.layer);
    if (index < 0) throw new Error('stale-layer');
    const otherOwners = draft.paths().filter(path => worldPath(path) && path !== operation.path)
      .filter(path => {
        const text = draft.read(path);
        return typeof text === 'string' && (parsed(text, path).extra_worlds || []).includes(operation.layer);
      });
    if (otherOwners.length) { const error = new Error('layer-in-use'); error.owners = otherOwners; throw error; }
    layers.splice(index, 1);
    changes = [{ path: operation.path, before: source, after: setExtraWorlds(source, layers) }];
    if (draft.paths().includes(operation.layer)) changes.push({ path: operation.layer,
      before: draft.read(operation.layer), after: null });
  } else throw new Error('invalid-spatial-operation');
  return { changes };
}

export async function applySpatialOperation({ draft, provider, runtime, operation, dependencies, current = () => true }) {
  const revision = draft.sourceRevision, prepared = prepareSpatialOperation(draft, operation);
  const candidate = provider?.restoreDocument ? provider.restoreDocument(draft.snapshot()) : WorkshopDocument.restore(draft.snapshot());
  candidate.apply(prepared.changes);
  spatialInventory(candidate, dependencies);
  const report = await runtime.validate(candidate.kind === 'mod' && !provider?.save ? candidate.archive() : null, candidate);
  if (!report?.accepted) { const error = new Error('runtime-validation-refused'); error.report = report; throw error; }
  if (!current() || draft.sourceRevision !== revision
    || prepared.changes.some(change => (draft.read(change.path) ?? null) !== change.before)) {
    throw new Error('stale-spatial-operation');
  }
  draft.apply(prepared.changes);
  return { changes: prepared.changes, report, inventory: spatialInventory(draft, dependencies) };
}
