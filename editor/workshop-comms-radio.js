import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const object = value => value && typeof value === 'object' && !Array.isArray(value);
const closed = (value, keys) => object(value) && Object.keys(value).every(key => keys.includes(key));
const bytes = value => new TextEncoder().encode(value).length;
const identity = value => typeof value === 'string' && value.trim().length > 0 && bytes(value) <= 128 && !/[\u0000-\u001f\u007f-\u009f]/u.test(value);
const positive = value => typeof value === 'number' && Number.isFinite(Math.fround(value)) && Math.fround(value) > 0;
function equipment(entity) {
  const system = authoredAndGeneratedSystems(entity).find(row => row.id === 'comms' && row.kind === 'comms');
  const consumer = entity.energy?.consumer?.find(row => row.system === 'comms');
  return positive(entity.comms?.range) && system && entity.console?.some(row => row.id === system.console)
    && positive(consumer?.continuous_cost_per_sec) && positive(consumer?.discrete_cost);
}
export function validateCommsRadio(radio, entity) {
  if (!equipment(entity)) throw new Error('Radio needs the installed Comms System, its owning Console, finite endpoint reach and positive continuous/discrete energy costs.');
  if (!closed(radio, ['system', 'keys', 'channel', 'request_reading_ticks', 'media', 'capture']) || radio.system !== 'comms'
    || (radio.media !== undefined && !['audio', 'audio_video'].includes(radio.media))
    || !Array.isArray(radio.keys === undefined ? [] : radio.keys) || !Array.isArray(radio.channel) || !radio.channel.length)
    throw new Error('Configure the canonical Comms owner and at least one supported radio channel.');
  if (radio.request_reading_ticks !== undefined && (!Number.isSafeInteger(radio.request_reading_ticks) || radio.request_reading_ticks <= 0 || radio.request_reading_ticks > 0xffffffff))
    throw new Error('Request reading requires positive whole logical ticks.');
  if (radio.capture !== undefined) {
    const capture = radio.capture, ticks = value => Number.isSafeInteger(value) && value > 0 && value <= 36000;
    if (!closed(capture, ['max_level', 'ticks_per_level', 'held_key_ticks']) || !Number.isSafeInteger(capture.max_level)
      || capture.max_level < 0 || capture.max_level > 3 || !ticks(capture.ticks_per_level)
      || (capture.held_key_ticks !== undefined && !ticks(capture.held_key_ticks)))
      throw new Error('Capture needs a closed equipment ceiling and positive bounded paid decode ticks.');
  }
  const keys = radio.keys === undefined ? [] : radio.keys, ids = new Set();
  if (keys.some(key => !identity(key)) || new Set(keys).size !== keys.length) throw new Error('Held protection keys require distinct bounded identities.');
  for (const channel of radio.channel) {
    if (!closed(channel, ['id', 'range', 'work_units', 'key', 'relay', 'protection_level']) || typeof channel.id !== 'string'
      || !/^[a-z0-9-]+$/.test(channel.id) || bytes(channel.id) > 128 || ids.has(channel.id)
      || (channel.relay !== undefined && typeof channel.relay !== 'boolean')
      || (channel.protection_level !== undefined && (!Number.isSafeInteger(channel.protection_level) || channel.protection_level < 1 || channel.protection_level > 3))
      || !positive(channel.range) || Math.fround(channel.range) > Math.fround(entity.comms.range)
      || !Number.isSafeInteger(channel.work_units) || channel.work_units <= 0 || channel.work_units > 0xffffffff
      || (channel.key !== undefined && (!identity(channel.key) || !keys.includes(channel.key))))
      throw new Error('Use unique channel ids, finite endpoint-bounded range, positive whole work units and actually held protection keys.');
    ids.add(channel.id);
  }
}
export function inspectCommsRadio(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path), entity = inspection.resolved.value;
  if (entity.comms?.radio !== undefined) validateCommsRadio(entity.comms.radio, entity);
  return { ...inspection, capable: !!equipment(entity), configured: entity.comms?.radio !== undefined,
    radio: structuredClone(entity.comms?.radio ?? { system: 'comms', keys: [], channel: [] }),
    endpointRange: entity.comms?.range,
    inherited: inspection.fields.some(row => (row.path === 'comms.radio' || row.path.startsWith('comms.radio.')) && row.owner !== path) };
}
export function guardCommsRadioReferences(_before, after) {
  if (after.comms?.radio !== undefined) validateCommsRadio(after.comms.radio, after);
}
const ordered = value => Array.isArray(value) ? value.map(ordered) : object(value)
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, ordered(value[key])])) : value;
function writeRadio(source, radio) {
  const before = parse(source), nl = source.includes('\r\n') ? '\r\n' : '\n';
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
  const spans = headers.filter(row => row.name === 'comms.radio' || row.name.startsWith('comms.radio.'));
  if (!spans.length && before.comms?.radio !== undefined) throw new Error('Expand inline radio source into ordinary TOML tables before editing.');
  let after = source;
  for (const row of spans.slice().reverse()) {
    const end = headers.find(next => next.start > row.start)?.start ?? source.length;
    const comments = source.slice(row.start, end).split(/\r?\n/).map(line => line.match(/#[^\r\n]*/)?.[0]).filter(Boolean).join(nl);
    after = after.slice(0, row.start) + (comments ? comments + nl : '') + after.slice(end);
  }
  const text = stringify({ comms: { radio } }).replace(/\r?\n/g, nl);
  after += (after.endsWith(nl) ? nl : nl + nl) + text;
  const parsed = parse(after), without = value => {
    const copy = structuredClone(value);
    if (copy.comms) { delete copy.comms.radio; if (!Object.keys(copy.comms).length) delete copy.comms; }
    return JSON.stringify(ordered(copy));
  };
  if (JSON.stringify(ordered(parsed.comms?.radio)) !== JSON.stringify(ordered(radio)) || without(before) !== without(parsed))
    throw new Error('Expand radio source into ordinary tables before editing; unrelated Comms settings must remain unchanged.');
  return after;
}
export function prepareCommsRadioOperation(draft, dependencies, operation) {
  const inspection = inspectCommsRadio(draft, dependencies, operation.path);
  if (inspection.inherited && !operation.materialise) throw new Error('Explicitly materialise inherited radio channels before editing.');
  validateCommsRadio(operation.value, inspection.resolved.value);
  const before = draft.read(operation.path), after = writeRadio(before, operation.value);
  return { changes: before === after ? [] : [{ path: operation.path, before, after }] };
}
export async function applyCommsRadioOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current,
    prepare: (captured, effective) => prepareCommsRadioOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectCommsRadio(candidate, effective, selected.path) });
}
