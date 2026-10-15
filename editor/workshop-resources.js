import { guardFittingReferences } from './workshop-fitting.js';
import { guardPortServices } from './workshop-ports.js';
import { guardRecoveryReferences } from './workshop-resource-recovery.js';
import { guardManagedStockBounds } from './workshop-stock-bindings.js';
import { guardProductionReferences } from './workshop-production.js';
import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const clone = value => structuredClone(value);
const quote = value => JSON.stringify(String(value));
const nlOf = source => source.includes('\r\n') ? '\r\n' : '\n';
const named = value => typeof value === 'string' && value.trim() !== '';
const integer = value => Number.isSafeInteger(value) && value >= 0;
const endpoint = value => value?.kind === 'torpedo_magazine' ? { kind: value.kind }
  : value?.kind === 'capacity' && named(value.id) ? { kind: value.kind, id: value.id }
    : (() => { throw new Error('Invalid resource endpoint.'); })();

function tableHeaders(source) {
  return [...source.matchAll(/^[ \t]*\[(\[?)([^\r\n]+?)\]\]?[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(row => ({ start: row.index, body: row.index + row[0].length, array: row[1] === '[', name: row[2].trim() }));
}
function table(source, name, id) {
  const headers = tableHeaders(source);
  for (let i = 0; i < headers.length; i += 1) {
    const item = headers[i], end = headers[i + 1]?.start ?? source.length;
    if (item.name !== name) continue;
    if (id !== undefined) {
      const parsed = parse(source.slice(item.start, end));
      const rows = name.split('.').reduce((value, key) => value?.[key], parsed);
      if (!rows?.some(row => row.id === id && row._remove !== true)) continue;
    }
    return { ...item, end };
  }
  return null;
}
function append(source, fragment) {
  const nl = nlOf(source), gap = !source.length || source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl;
  return source + gap + fragment.trim().replace(/\n/g, nl) + nl;
}
function scalar(source, record, key, value) {
  const body = source.slice(record.body, record.end), regex = new RegExp(`^([ \t]*${key}[ \t]*=[ \t]*)([^\r\n#]*)(.*)$`, 'm');
  const match = regex.exec(body), serialized = typeof value === 'string' ? quote(value) : String(value);
  if (!match) {
    const before = source.slice(0, record.end), nl = nlOf(source);
    return before + (before.endsWith(nl) ? '' : nl) + `${key} = ${serialized}${nl}` + source.slice(record.end);
  }
  // Scalars in this form are integer or basic strings. Find a comment only
  // outside a quoted string so ids/labels containing # retain their bytes.
  const line = body.slice(match.index).split(/\r?\n/, 1)[0];
  let at = match[1].length, mark = null, escaped = false, end = line.length;
  for (let i = at; i < line.length; i += 1) {
    const c = line[i];
    if (escaped) { escaped = false; continue; }
    if (mark === '"' && c === '\\') { escaped = true; continue; }
    if (mark && c === mark) mark = null;
    else if (!mark && (c === '"' || c === "'")) mark = c;
    if (!mark && c === '#') { end = i; break; }
  }
  while (end > at && /\s/.test(line[end - 1])) end -= 1;
  const start = record.body + match.index + at;
  return source.slice(0, start) + serialized + source.slice(record.body + match.index + end);
}
function requireMaterialise(inspection, prefix, materialise) {
  const inherited = inspection.fields.filter(row => row.path === prefix || row.path.startsWith(prefix + '.')).filter(row => row.owner !== inspection.path);
  if (inherited.length && !materialise) throw new Error('Explicitly materialise inherited resource settings before editing.');
}
function putEndpoint(source, path, value) {
  const name = path.map(key => /^[A-Za-z0-9_-]+$/.test(key) ? key : quote(key)).join('.');
  const belongs = header => header.name === name || header.name.startsWith(name + '.');
  const headers = tableHeaders(source), index = headers.findIndex(belongs);
  const existing = index < 0 ? null : { start: headers[index].start,
    end: headers.slice(index + 1).find(header => !belongs(header))?.start ?? source.length };
  let fragment = stringify(path.slice().reverse().reduce((child, key) => ({ [key]: child }), value)).trim();
  // The selected binding is atomic; retain unrelated bytes and parent settings.
  const first = tableHeaders(fragment).find(belongs);
  if (!first) throw new Error('Resource endpoint source is unavailable.');
  fragment = fragment.slice(first.start);
  if (existing) {
    const nl = nlOf(source);
    return source.slice(0, existing.start) + fragment.replace(/\n/g, nl) + nl + source.slice(existing.end);
  }
  const document = parse(source);
  let local = document;
  for (const part of path) local = local?.[part];
  if (local !== undefined) throw new Error('Expand inline resource endpoints into TOML tables before editing.');
  return append(source, fragment);
}

export function inspectResources(draft, dependencies, path) {
  const composition = inspectEntityComposition(draft, dependencies, path), value = composition.resolved.value;
  return { ...composition, capacities: clone(value.infrastructure?.capacity || []), services: clone(value.resource_services || {}),
    magazine: clone(value.torpedoes || null), umbilical: clone(value.umbilical || null) };
}

export function prepareResourceOperation(draft, dependencies, operation) {
  const inspection = inspectResources(draft, dependencies, operation.path), before = draft.read(operation.path);
  let after = before;
  if (operation.type === 'capacity-set') {
    if (!named(operation.id) || !integer(operation.amount) || !integer(operation.ceiling) || operation.amount > operation.ceiling)
      throw new Error('Capacity requires a name and whole quantities between zero and its limit.');
    guardManagedStockBounds(inspection.resolved.value, { kind: 'capacity', id: operation.id }, operation.amount, operation.ceiling);
    const prefix = `infrastructure.capacity[id=${operation.id}]`;
    requireMaterialise(inspection, prefix, operation.materialise);
    let record = table(after, 'infrastructure.capacity', operation.id);
    if (!record) {
      const local = parse(after).infrastructure?.capacity;
      if (local && (!Array.isArray(local) || local.some(row => row.id === operation.id && row._remove !== true))) throw new Error('Expand inline capacities before editing.');
      after = append(after, `[[infrastructure.capacity]]\nid = ${quote(operation.id)}`);
    }
    const fields = { amount: operation.amount, ceiling: operation.ceiling,
      ...(operation.label === undefined ? {} : { label: operation.label }) };
    for (const [key, value] of Object.entries(fields)) after = scalar(after, table(after, 'infrastructure.capacity', operation.id), key, value);
  } else if (operation.type === 'magazine-set') {
    if (!inspection.magazine || !integer(operation.count) || operation.count > 4294967295) throw new Error('Magazine quantity must fit an existing torpedo magazine.');
    guardManagedStockBounds(inspection.resolved.value, { kind: 'torpedo_magazine' }, operation.initial_count ?? inspection.magazine.initial_count ?? operation.count, operation.count);
    requireMaterialise(inspection, 'torpedoes.count', operation.materialise);
    if (!table(after, 'torpedoes')) {
      if (parse(after).torpedoes) throw new Error('Expand inline torpedoes into TOML tables before editing.');
      after = append(after, '[torpedoes]');
    }
    after = scalar(after, table(after, 'torpedoes'), 'count', operation.count);
    if (operation.initial_count !== undefined) {
      if (!integer(operation.initial_count) || operation.initial_count > operation.count) throw new Error('Initial magazine stock must fit its ammunition capacity.');
      after = scalar(after, table(after, 'torpedoes'), 'initial_count', operation.initial_count);
    }
  } else if (operation.type === 'service-set') {
    if (!named(operation.name)) throw new Error('Resource service requires a name.');
    const address = /[.\[\]= ]/.test(operation.name) ? quote(operation.name) : operation.name;
    requireMaterialise(inspection, `resource_services.${address}`, operation.materialise);
    after = putEndpoint(after, ['resource_services', operation.name], endpoint(operation.endpoint));
  } else if (operation.type === 'link-set') {
    if (!inspection.umbilical) throw new Error('Entity has no transfer umbilical.');
    requireMaterialise(inspection, 'umbilical.link', operation.materialise);
    if (!operation.provider && !named(operation.service)) throw new Error('Choose a named service or an explicit provider.');
    after = putEndpoint(after, ['umbilical', 'link'], operation.provider
      ? { provider: endpoint(operation.provider) } : { service: operation.service });
  } else throw new Error('Unsupported resource operation.');
  // Pure preparation never lands an invalid draft. The authoritative runtime
  // validation below checks endpoint references, System admission and content.
  parse(after);
  if (inspection.resolved.value.fitting || inspection.resolved.value.production || inspection.resolved.value.recovery || inspection.resolved.value.recoverable || inspection.resolved.value.port_service) {
    const candidate = { paths: () => draft.paths(), isBinary: path => draft.isBinary(path), read: path => path === operation.path ? after : draft.read(path) };
    const proposed = inspectResources(candidate, dependencies, operation.path).resolved.value;
    guardFittingReferences(inspection.resolved.value, proposed);
    guardProductionReferences(inspection.resolved.value, proposed);
    guardRecoveryReferences(inspection.resolved.value, proposed);
    guardPortServices(inspection.resolved.value, proposed);
  }
  return { changes: [{ path: operation.path, before, after }] };
}

export async function applyResourceOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = clone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-resource-operation',
    prepare: (captured, effective) => prepareResourceOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectResources(candidate, effective, selected.path) });
}
