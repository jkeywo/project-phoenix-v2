import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { WorkshopDocument } from './workshop-document.js';

const quote = value => JSON.stringify(String(value));
const newlineOf = source => source.includes('\r\n') ? '\r\n' : '\n';
const ident = value => typeof value === 'string' && value.trim() === value && /^[A-Za-z0-9][A-Za-z0-9_-]*$/.test(value);

function headers(source) {
  return [...source.matchAll(/^[ \t]*\[\[([^\]\r\n]+)\]\][ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ name: match[1].trim(), start: match.index, body: match.index + match[0].length }));
}

function blocks(source, name, parent = null) {
  const all = headers(source), matches = [];
  for (let i = 0; i < all.length; i += 1) {
    const item = all[i];
    if (item.name !== name) continue;
    const end = all.slice(i + 1).find(next => parent
      ? next.name === name || next.name === parent || !next.name.startsWith(`${parent}.`)
      : next.name === name || !next.name.startsWith(`${name}.`))?.start ?? source.length;
    matches.push({ ...item, end });
  }
  return matches;
}

function valueEnd(source, start, limit) {
  let i = start, mark = null, escaped = false, square = 0, curly = 0;
  for (; i < limit; i += 1) {
    const c = source[i];
    if (mark) { if (escaped) escaped = false; else if (c === '\\' && mark === '"') escaped = true; else if (c === mark) mark = null; }
    else if (c === '"' || c === "'") mark = c;
    else if (c === '[') square += 1; else if (c === ']') square -= 1;
    else if (c === '{') curly += 1; else if (c === '}') curly -= 1;
    else if ((c === '\r' || c === '\n' || c === '#') && !square && !curly) break;
  }
  while (i > start && /\s/.test(source[i - 1])) i -= 1;
  return i;
}

function entry(source, name, id, key = 'id') {
  for (const block of blocks(source, name, name.split('.')[0])) {
    const text = source.slice(block.start, block.end);
    let parsed;
    try { parsed = parse(text); } catch { continue; }
    const rows = name.split('.').reduce((value, part) => value?.[part], parsed);
    const row = Array.isArray(rows) ? rows[0] : null;
    if (row && String(row[key]) === String(id)) return { ...block, row };
  }
  return null;
}

function tomlValue(value) {
  if (typeof value === 'string') return quote(value);
  if (typeof value === 'boolean' || (typeof value === 'number' && Number.isFinite(value))) return String(value);
  if (Array.isArray(value)) return `[${value.map(tomlValue).join(', ')}]`;
  throw new Error('unsupported-ship-value');
}

function setKey(source, block, key, value) {
  const bodyEnd = headers(source).find(header => header.start >= block.body && header.start < block.end)?.start ?? block.end;
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = new RegExp(`^[ \\t]*${escaped}[ \\t]*=[ \\t]*`, 'm').exec(source.slice(block.body, bodyEnd));
  if (match) {
    const start = block.body + match.index + match[0].length;
    return source.slice(0, start) + tomlValue(value) + source.slice(valueEnd(source, start, bodyEnd));
  }
  const nl = newlineOf(source), line = `${key} = ${tomlValue(value)}${nl}`;
  return source.slice(0, bodyEnd) + line + source.slice(bodyEnd);
}

function removeKey(source, block, key) {
  const bodyEnd = headers(source).find(header => header.start >= block.body && header.start < block.end)?.start ?? block.end;
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = new RegExp(`^[ \\t]*${escaped}[ \\t]*=[^\\r\\n]*(?:\\r?\\n|$)`, 'm').exec(source.slice(block.body, bodyEnd));
  return match ? source.slice(0, block.body + match.index) + source.slice(block.body + match.index + match[0].length) : source;
}

function append(source, value) {
  const nl = newlineOf(source), gap = !source.length || source.endsWith(`${nl}${nl}`) ? '' : source.endsWith(nl) ? nl : `${nl}${nl}`;
  return source + gap + stringify(value).trim().replace(/\n/g, nl) + nl;
}

function appendNestedTable(source, name, value) {
  const nl = newlineOf(source), serialized = stringify(name.split('.').reverse().reduce((child, key) => ({ [key]: child }), [value]))
    .trim().replace(/\n/g, nl);
  const marker = `[[${name}]]`, start = serialized.indexOf(marker);
  if (start < 0) throw new Error('ship-source-unavailable');
  const fragment = serialized.slice(start), gap = !source.length || source.endsWith(`${nl}${nl}`) ? '' : source.endsWith(nl) ? nl : `${nl}${nl}`;
  return source + gap + fragment + nl;
}

function remove(source, block) { return source.slice(0, block.start) + source.slice(block.end); }
function move(source, list, index, direction) {
  const other = index + direction;
  if (other < 0 || other >= list.length) throw new Error('ship-order-boundary');
  const first = list[Math.min(index, other)], second = list[Math.max(index, other)];
  const a = source.slice(first.start, first.end), middle = source.slice(first.end, second.start), b = source.slice(second.start, second.end);
  return source.slice(0, first.start) + b + middle + a + source.slice(second.end);
}

function finding(path, source, token, message) {
  const at = Math.max(0, source.indexOf(token));
  const error = new Error(message);
  error.report = { accepted: false, findings: [{ severity: 'error', category: 'ship-authoring', message,
    file: path, line: source.slice(0, at).split(/\r?\n/).length }] };
  return error;
}

function ownedFinding(inspection, section, id, message) {
  const prefix = `${section}[id=${id}]`;
  const owner = inspection.composition.fields.find(field => field.path === `${prefix}.id`
    || field.path.startsWith(`${prefix}.`))?.owner || inspection.path;
  const source = inspection.composition.inventory.find(row => row.path === owner)?.text || '';
  return finding(owner, source, quote(id), message);
}

function provenanceFinding(inspection, prefix, token, message) {
  const owner = inspection.composition.fields.find(field => field.path === prefix || field.path.startsWith(`${prefix}.`))?.owner || inspection.path;
  const source = inspection.composition.inventory.find(row => row.path === owner)?.text || '';
  return finding(owner, source, token, message);
}

function verify(path, source, schema, value = parse(source)) {
  const stations = value.station || [], systems = value.system || [];
  const stationIds = new Set(), systemIds = new Set();
  for (const station of stations) {
    if (!ident(station.id) || station.id === 'core' || stationIds.has(station.id)) throw finding(path, source, `id = ${quote(station.id)}`, 'Station ids must be unique runtime identifiers and cannot be core.');
    stationIds.add(station.id);
  }
  for (const system of systems) {
    if (!ident(system.id) || systemIds.has(system.id)) throw finding(path, source, `id = ${quote(system.id)}`, 'System ids must be unique runtime identifiers.');
    systemIds.add(system.id);
    if (!schema.system_kinds.includes(system.kind)) throw finding(path, source, `kind = ${quote(system.kind)}`, `Unknown runtime System kind: ${system.kind}`);
    if (system.station != null && !stationIds.has(system.station)) throw finding(path, source, `station = ${quote(system.station)}`, `System ${system.id} names an unknown Station.`);
    if (system.station == null && !system.ai_only) throw finding(path, source, `id = ${quote(system.id)}`, `Ownerless System ${system.id} must be AI-only.`);
  }
  for (const station of stations) for (const rating of station.rating || []) for (const reference of rating.automated_systems || []) {
    const system = systems.find(candidate => candidate.id === reference);
    if (!system || system.station !== station.id) throw finding(path, source, quote(reference), `Rating ${rating.name} may automate only Systems owned by Station ${station.id}.`);
  }
  for (const doctrine of value.behaviour?.doctrine || []) if (doctrine.directive_kind != null && !schema.directive_kinds.includes(doctrine.directive_kind)) {
    throw finding(path, source, `directive_kind = ${quote(doctrine.directive_kind)}`, `Unknown runtime Directive kind: ${doctrine.directive_kind}`);
  }
}

function localIds(source, section) { return (parse(source)[section] || []).map(row => String(row.id)); }
function rowsWithOwners(inspection, section) {
  const rows = inspection.resolved.value[section] || [];
  return rows.map(row => ({ ...structuredClone(row), owner: inspection.fields.find(field => field.path.startsWith(`${section}[id=${row.id}]`))?.owner || inspection.path,
    local: localIds(inspection.inventory.find(item => item.path === inspection.path).text, section).includes(String(row.id)) }));
}

export function inspectShipAuthoring(draft, dependencies, path) {
  const composition = inspectEntityComposition(draft, dependencies, path);
  const tags = composition.resolved.value.tags || [];
  if (!tags.includes('ship')) throw new Error('entity-is-not-a-ship');
  const own = parse(draft.read(path)), localDoctrine = new Set((own.behaviour?.doctrine || []).map(row => String(row.id)));
  const doctrines = (composition.resolved.value.behaviour?.doctrine || []).map(row => {
    const owner = composition.fields.find(field => field.path === `behaviour.doctrine[id=${row.id}].id`
      || field.path.startsWith(`behaviour.doctrine[id=${row.id}].`))?.owner || path;
    return { ...structuredClone(row), owner, local: localDoctrine.has(String(row.id)) };
  });
  return { path, stations: rowsWithOwners(composition, 'station'), systems: rowsWithOwners(composition, 'system'), doctrines, composition };
}

export function prepareShipOperation(draft, dependencies, schema, operation) {
  const inspection = inspectShipAuthoring(draft, dependencies, operation.path);
  const before = draft.read(operation.path); let after = before;
  if (operation.type === 'station-add') after = append(after, { station: [{ id: operation.id, name: operation.name || operation.id,
    description: operation.description || operation.id, rank: operation.rank || '', console: operation.console || undefined }] });
  else if (operation.type === 'station-set' || operation.type === 'station-update') {
    const fields = operation.type === 'station-set' ? { [operation.field]: operation.value } : operation.fields;
    for (const [field, value] of Object.entries(fields)) { const block = entry(after, 'station', operation.id); if (!block) throw ownedFinding(inspection, 'station', operation.id, `Station ${operation.id} is included from another source; materialise it before editing.`);
      after = value === undefined ? removeKey(after, block, field) : setKey(after, block, field, value); }
  }
  else if (operation.type === 'station-remove') { const block = entry(after, 'station', operation.id); if (!block) throw ownedFinding(inspection, 'station', operation.id, `Station ${operation.id} is included from another source; materialise it before removing it.`); after = remove(after, block); }
  else if (operation.type === 'station-move') { const list = blocks(after, 'station', 'station'); const index = localIds(after, 'station').indexOf(operation.id);
    if (index < 0) throw ownedFinding(inspection, 'station', operation.id, `Station ${operation.id} is included from another source; materialise it before reordering it.`);
    after = move(after, list, index, operation.direction); }
  else if (operation.type === 'system-add') after = append(after, { system: [{ id: operation.id, kind: operation.kind,
    ...(operation.station ? { station: operation.station } : { ai_only: true }) }] });
  else if (operation.type === 'system-set' || operation.type === 'system-update') {
    const fields = operation.type === 'system-set' ? { [operation.field]: operation.value } : operation.fields;
    for (const [field, value] of Object.entries(fields)) { const block = entry(after, 'system', operation.id); if (!block) throw ownedFinding(inspection, 'system', operation.id, `System ${operation.id} is included from another source; materialise it before editing.`);
      after = value === undefined ? removeKey(after, block, field) : setKey(after, block, field, value); }
  }
  else if (operation.type === 'system-remove') { const block = entry(after, 'system', operation.id); if (!block) throw ownedFinding(inspection, 'system', operation.id, `System ${operation.id} is included from another source; materialise it before removing it.`); after = remove(after, block); }
  else if (operation.type === 'system-move') { const list = blocks(after, 'system', 'system'); const index = localIds(after, 'system').indexOf(operation.id);
    if (index < 0) throw ownedFinding(inspection, 'system', operation.id, `System ${operation.id} is included from another source; materialise it before reordering it.`);
    after = move(after, list, index, operation.direction); }
  else if (operation.type === 'rating-add') { const station = entry(after, 'station', operation.station); if (!station) throw ownedFinding(inspection, 'station', operation.station, `Station ${operation.station} is included from another source; materialise it before adding a rating.`);
    const nl = newlineOf(after), fragment = `${nl}[[station.rating]]${nl}name = ${quote(operation.name)}${nl}automated_systems = []${nl}`;
    after = after.slice(0, station.end) + fragment + after.slice(station.end); }
  else if (operation.type === 'rating-set-systems' || operation.type === 'rating-update' || operation.type === 'rating-remove') {
    const station = entry(after, 'station', operation.station); if (!station) throw ownedFinding(inspection, 'station', operation.station, `Station ${operation.station} is included from another source; materialise it before editing its ratings.`);
    const relative = after.slice(station.start, station.end), rating = entry(relative, 'station.rating', operation.rating, 'name');
    if (!rating) throw provenanceFinding(inspection, `station[id=${operation.station}].rating[name=${operation.rating}]`, quote(operation.rating), `Rating ${operation.rating} is included from another source; materialise it before editing.`); const absolute = { ...rating, start: station.start + rating.start, body: station.start + rating.body, end: station.start + rating.end };
    if (operation.type === 'rating-remove') after = remove(after, absolute);
    else {
      after = setKey(after, absolute, 'automated_systems', operation.systems);
      if (operation.type === 'rating-update' && operation.name !== operation.rating) {
        const updatedStation = entry(after, 'station', operation.station), updatedRelative = after.slice(updatedStation.start, updatedStation.end);
        const updatedRating = entry(updatedRelative, 'station.rating', operation.rating, 'name');
        after = setKey(after, { ...updatedRating, start: updatedStation.start + updatedRating.start,
          body: updatedStation.start + updatedRating.body, end: updatedStation.start + updatedRating.end }, 'name', operation.name);
      }
    }
  } else if (operation.type === 'rating-move') {
    const station = entry(after, 'station', operation.station); if (!station) throw ownedFinding(inspection, 'station', operation.station, `Station ${operation.station} is included from another source; materialise it before reordering ratings.`);
    const relative = after.slice(station.start, station.end), list = blocks(relative, 'station.rating', 'station');
    const names = station.row.rating?.map(row => String(row.name)) || [], index = names.indexOf(operation.rating);
    if (index < 0) throw provenanceFinding(inspection, `station[id=${operation.station}].rating[name=${operation.rating}]`, quote(operation.rating), `Rating ${operation.rating} is included from another source; materialise it before reordering it.`);
    after = after.slice(0, station.start) + move(relative, list, index, operation.direction) + after.slice(station.end);
  } else if (operation.type === 'doctrine-add') {
    const row = { id: operation.id, base_priority: operation.base_priority };
    if (operation.kind !== 'None') row.directive_kind = operation.kind;
    Object.assign(row, operation.references || {});
    after = appendNestedTable(after, 'behaviour.doctrine', row);
  } else if (operation.type === 'doctrine-update') {
    let block = entry(after, 'behaviour.doctrine', operation.id); if (!block) throw ownedFinding(inspection, 'behaviour.doctrine', operation.id, `Doctrine ${operation.id} is included from another source; materialise it before editing.`);
    const referenceFields = ['directive_anchors', 'directive_loop', 'directive_target', 'directive_anchor', 'directive_dock_target',
      'directive_hail_target', 'directive_scan_target', 'directive_operate_target', 'directive_order_target', 'directive_order_route'];
    for (const field of referenceFields) { block = entry(after, 'behaviour.doctrine', operation.id); after = removeKey(after, block, field); }
    block = entry(after, 'behaviour.doctrine', operation.id);
    after = operation.kind === 'None' ? removeKey(after, block, 'directive_kind') : setKey(after, block, 'directive_kind', operation.kind);
    block = entry(after, 'behaviour.doctrine', operation.id); after = setKey(after, block, 'base_priority', operation.base_priority);
    for (const [field, value] of Object.entries(operation.references || {})) { block = entry(after, 'behaviour.doctrine', operation.id); after = setKey(after, block, field, value); }
  } else if (operation.type === 'doctrine-remove') {
    const block = entry(after, 'behaviour.doctrine', operation.id); if (!block) throw ownedFinding(inspection, 'behaviour.doctrine', operation.id, `Doctrine ${operation.id} is included from another source; materialise it before removing it.`);
    after = remove(after, block);
  } else if (operation.type === 'doctrine-move') {
    const list = blocks(after, 'behaviour.doctrine', 'behaviour'), ids = parse(after).behaviour?.doctrine?.map(row => String(row.id)) || [];
    const index = ids.indexOf(operation.id); if (index < 0) throw ownedFinding(inspection, 'behaviour.doctrine', operation.id, `Doctrine ${operation.id} is included from another source; materialise it before reordering it.`);
    after = move(after, list, index, operation.direction);
  }
  else throw new Error('invalid-ship-operation');
  const candidate = WorkshopDocument.restore(draft.snapshot());
  candidate.apply([{ path: operation.path, before, after }]);
  const effectiveValue = inspectShipAuthoring(candidate, dependencies, operation.path).composition.resolved.value;
  verify(operation.path, after, schema, effectiveValue);
  return { inspection, changes: [{ path: operation.path, before, after }] };
}

export async function applyShipOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const revision = draft.sourceRevision, effective = dependencies || await runtime.dependencies(), schema = await runtime.shipSchema();
  const prepared = prepareShipOperation(draft, effective, schema, operation);
  const candidate = provider?.restoreDocument ? provider.restoreDocument(draft.snapshot()) : WorkshopDocument.restore(draft.snapshot());
  candidate.apply(prepared.changes);
  inspectShipAuthoring(candidate, effective, operation.path);
  const report = await runtime.validate(candidate.kind === 'mod' && !provider?.save ? candidate.archive() : null, candidate);
  if (!report?.accepted) { const error = new Error('runtime-validation-refused'); error.report = report; throw error; }
  if (!current() || draft.sourceRevision !== revision || draft.read(operation.path) !== prepared.changes[0].before) throw new Error('stale-ship-operation');
  draft.apply(prepared.changes);
  return { report, inspection: inspectShipAuthoring(draft, effective, operation.path) };
}
