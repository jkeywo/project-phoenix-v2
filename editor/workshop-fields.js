import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
const roots = ['shape', 'field', 'signal_material'];
const closed = (value, keys) => value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).every(key => keys.includes(key));
const finite = value => typeof value === 'number' && Number.isFinite(Math.fround(value));
const positive = value => finite(value) && Math.fround(value) > 0;
const nonnegative = value => finite(value) && value >= 0;
export function inspectFields(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  return { ...inspection, environment: Object.fromEntries(roots.map(key => [key, structuredClone(inspection.resolved.value[key] || null)])) };
}
export function validateFields(environment, entity = {}) {
  if (!closed(environment, roots)) throw new Error('Use only supported shape, field and signal-material declarations.');
  const { shape, field, signal_material: material } = environment;
  if (shape) {
    if (shape.type === 'sphere' ? !closed(shape, ['type', 'radius']) || !positive(shape.radius)
      : shape.type === 'box' ? !closed(shape, ['type', 'half_extents', 'yaw']) || !Array.isArray(shape.half_extents) || shape.half_extents.length !== 3 || !shape.half_extents.every(positive) || (shape.yaw !== undefined && !finite(shape.yaw))
      : shape.type === 'torus' ? !closed(shape, ['type', 'inner_radius', 'outer_radius']) || !nonnegative(shape.inner_radius) || !positive(shape.outer_radius) || Math.fround(shape.outer_radius) <= Math.fround(shape.inner_radius)
      : true) throw new Error('Choose supported finite geometry with positive dimensions.');
  } else if (field || material || entity.effects && Object.keys(entity.effects).length) throw new Error('Fields, signal materials and Region effects require an explicit physical shape.');
  if (field) {
    if (!closed(field, ['falloff', 'strength', 'sensor_absorption_per_unit', 'comms_absorption_per_unit', 'evolution']) || !['step', 'linear', 'smooth'].includes(field.falloff)
      || ![field.strength, field.sensor_absorption_per_unit, field.comms_absorption_per_unit].every(nonnegative)) throw new Error('Choose closed finite field falloff, strength and nonnegative signal absorption.');
    const curve = field.evolution;
    if (curve && (!closed(curve, ['duration_secs', 'translation', 'final_scale', 'final_strength']) || !positive(curve.duration_secs) || !positive(curve.final_scale) || !nonnegative(curve.final_strength)
      || !Array.isArray(curve.translation) || curve.translation.length !== 3 || !curve.translation.every(finite))) throw new Error('Evolution requires positive duration/scale, finite translation and nonnegative final strength.');
  }
  if (material && (!closed(material, ['sensor_absorption_per_unit', 'comms_absorption_per_unit', 'sensor_opaque', 'comms_opaque']) || ![material.sensor_absorption_per_unit, material.comms_absorption_per_unit].every(nonnegative)
    || typeof material.sensor_opaque !== 'boolean' || typeof material.comms_opaque !== 'boolean')) throw new Error('Choose explicit signal-specific opacity and finite nonnegative material absorption.');
}
function replaceRoot(source, key, value) {
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)].map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
  const spans = headers.filter(row => row.name === key || row.name.startsWith(key + '.')).map(row => {
    let end = headers.find(next => next.start > row.start)?.start ?? source.length;
    const tail = source.slice(row.start, end).match(/(?:^|\n)((?:[ \t]*(?:#[^\r\n]*)?\r?\n)+)$/)?.[1];
    if (tail?.includes('#')) end -= tail.length; return { start: row.start, end };
  });
  if (!spans.length && parse(source)[key] !== undefined) throw new Error('Expand inline environment declarations into TOML tables before editing.');
  if (!spans.length && !value) return source;
  const nl = source.includes('\r\n') ? '\r\n' : '\n', text = value ? stringify({ [key]: value }).trim().replace(/\n/g, nl) + nl : '';
  if (!spans.length) return source + (source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl) + text;
  let after = source; for (const span of spans.slice().reverse()) after = after.slice(0, span.start) + after.slice(span.end);
  return after.slice(0, spans[0].start) + text + after.slice(spans[0].start);
}
export function prepareFieldsOperation(draft, dependencies, operation) {
  const inspection = inspectFields(draft, dependencies, operation.path), before = draft.read(operation.path);
  for (const key of roots) {
    const inherited = inspection.fields.some(row => (row.path === key || row.path.startsWith(key + '.')) && row.owner !== operation.path);
    if (inherited && !operation.materialise) throw new Error('Explicitly materialise inherited environment declarations before editing.');
    if (inherited && !operation.value[key]) throw new Error('An inherited environment cannot be removed by deleting a local override. Edit its source/include, or set field strength to zero.');
  }
  validateFields(operation.value, inspection.resolved.value);
  let after = before; for (const key of roots) after = replaceRoot(after, key, operation.value[key]);
  return { changes: [{ path: operation.path, before, after }] };
}
export async function applyFieldsOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-field-operation',
    prepare: (captured, effective) => prepareFieldsOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectFields(candidate, effective, selected.path) });
}
