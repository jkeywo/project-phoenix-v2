import { parse } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const object = value => value && typeof value === 'object' && !Array.isArray(value);
export function validateHelmProjection(value) {
  if (value === false) return;
  if (!object(value) || Object.keys(value).some(key => key !== 'horizon_secs')
    || typeof value.horizon_secs !== 'number' || !Number.isFinite(Math.fround(value.horizon_secs))
    || Math.fround(value.horizon_secs) <= 0) throw new Error('Choose a finite positive Helm projection horizon, or disable projection.');
}
export function inspectHelmProjection(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  const helm = inspection.resolved.value.helm_console;
  const projection = helm?.projection;
  if (projection !== undefined) validateHelmProjection(projection);
  return { ...inspection, capable: object(helm), projection: structuredClone(projection ?? false),
    inherited: inspection.fields.some(row => (row.path === 'helm_console.projection'
      || row.path.startsWith('helm_console.projection.')) && row.owner !== path) };
}

// This bounded field has only one numeric member. Preserve all other Helm
// source, including comments and local radar/movement settings, byte for byte.
function writeProjection(source, value) {
  const own = parse(source), newline = source.includes('\r\n') ? '\r\n' : '\n';
  const literal = value === false ? 'false' : `{ horizon_secs = ${value.horizon_secs} }`;
  const headers = text => [...text.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ name: match[1].replace(/\]$/, '').trim(), start: match.index, body: match.index + match[0].length, text: match[0] }));
  let rows = headers(source);
  if (own.helm_console && !rows.some(row => row.name === 'helm_console' || row.name.startsWith('helm_console.'))) {
    throw new Error('Expand inline or dotted Helm source into tables before editing its projection.');
  }
  const nested = rows.find(row => row.name === 'helm_console.projection');
  if (nested) {
    const end = rows.find(row => row.start > nested.start)?.start ?? source.length;
    const body = source.slice(nested.body, end);
    const comments = [nested.text.match(/#[^\r\n]*/)?.[0], ...body.split(/\r?\n/)
      .map(line => /^[ \t]*#/.test(line) ? line : line.match(/#[^\r\n]*/)?.[0])]
      .filter(line => line !== undefined).join(newline);
    source = source.slice(0, nested.start) + (comments ? comments + newline : '') + source.slice(end);
    rows = headers(source);
  }
  const parent = rows.find(row => row.name === 'helm_console');
  if (parent) {
    const end = rows.find(row => row.start > parent.start)?.start ?? source.length;
    const body = source.slice(parent.body, end);
    const field = /^[ \t]*projection[ \t]*=[ \t]*(false|\{[^{}\r\n]*\})/m.exec(body);
    if (field) {
      const at = parent.body + field.index + field[0].length - field[1].length;
      source = source.slice(0, at) + literal + source.slice(at + field[1].length);
    } else {
      if (!nested && own.helm_console?.projection !== undefined) throw new Error('Expand the Helm projection into a supported table or inline field before editing.');
      const gap = source.slice(0, parent.body).endsWith('\n') ? '' : newline;
      source = source.slice(0, parent.body) + `${gap}projection = ${literal}${newline}` + source.slice(parent.body);
    }
  } else {
    if (!nested && own.helm_console?.projection !== undefined) throw new Error('Expand the Helm projection into a supported table or inline field before editing.');
    const gap = !source || source.endsWith(newline + newline) ? '' : source.endsWith(newline) ? newline : newline + newline;
    source += `${gap}[helm_console]${newline}projection = ${literal}${newline}`;
  }
  const parsed = parse(source);
  // Refuse unusual source layouts if a header-like string caused the bounded
  // writer to touch anything except this exact policy.
  const withoutProjection = value => {
    const copy = structuredClone(value);
    if (object(copy.helm_console)) {
      delete copy.helm_console.projection;
      if (!Object.keys(copy.helm_console).length) delete copy.helm_console;
    }
    const ordered = row => Array.isArray(row) ? row.map(ordered) : object(row)
      ? Object.fromEntries(Object.keys(row).sort().map(key => [key, ordered(row[key])])) : row;
    return JSON.stringify(ordered(copy));
  };
  if (JSON.stringify(parsed.helm_console?.projection) !== JSON.stringify(value)
    || withoutProjection(own) !== withoutProjection(parsed)) throw new Error('Expand the Helm projection into ordinary tables before editing.');
  return source;
}

export function prepareHelmProjectionOperation(draft, dependencies, operation) {
  validateHelmProjection(operation.value);
  const inspection = inspectHelmProjection(draft, dependencies, operation.path);
  if (!inspection.capable) throw new Error('Choose an actual Helm configuration first.');
  if (inspection.inherited && !operation.materialise) throw new Error('Explicitly materialise the inherited Helm projection before editing.');
  const before = draft.read(operation.path), after = writeProjection(before, operation.value);
  return { changes: before === after ? [] : [{ path: operation.path, before, after }] };
}
export async function applyHelmProjectionOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current,
    prepare: (captured, effective) => prepareHelmProjectionOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectHelmProjection(candidate, effective, selected.path) });
}
