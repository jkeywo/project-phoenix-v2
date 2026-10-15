import { scanPublicationSource } from '../gui/scan-source.js';
import { prunePublicationFeedsSource } from './workshop-stock-bindings.js';
import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
import { prepareShipOperation } from './workshop-ship-authoring.js';

export const sensingRoles = {
  focused: { kind: 'sensors', fields: ['work_secs', 'history_limit', 'unfinished_retention_secs'] },
  broad: { kinds: ['helm_radar', 'tactical_radar', 'sensor_radar'], fields: ['sample_interval_secs', 'identity_secs', 'classification_secs', 'assessment_secs', 'position_error', 'drift_error_per_sec', 'track_retention_secs'] },
  passive: { kind: 'passive_receiver', fields: ['sample_interval_secs', 'min_signal_strength', 'bearing_error_rad'] },
};
export const initialPassiveTree = () => ({ root: 'passive', tree: [{ id: 'passive', initial: 'receive', state: [{ id: 'receive', task: [
  { id: 'operate-capable-receiver', action: { kind: 'write', channel: 'operate', value: { source: 'fact', name: 'can_operate' } } },
] }] }] });
const closed = (value, keys) => value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).every(key => keys.includes(key));
const finite = value => typeof value === 'number' && Number.isFinite(Math.fround(value)) && Math.fround(value) >= 0;
const positive = value => finite(value) && Math.fround(value) > 0;
export function inspectSensing(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  return { ...inspection, sensing: structuredClone(inspection.resolved.value.sensing || null), systems: authoredAndGeneratedSystems(inspection.resolved.value) };
}
export function validateSensing(profile, value) {
  if (!closed(profile, [...Object.keys(sensingRoles), 'control', 'sharing']) || (profile.sharing != null && !Array.isArray(profile.sharing)) || Object.keys(sensingRoles).some(role => profile[role] != null && !Array.isArray(profile[role]))
    || !Object.keys(sensingRoles).some(role => profile[role]?.length)) throw new Error('Sensing requires closed instrument profiles and instrument arrays.');
  const systems = authoredAndGeneratedSystems(value), seen = new Set();
  const shared = new Set();
  for (const row of profile.sharing || []) {
    if (!closed(row, ['system', 'share_own_faction', 'receive']) || shared.has(row.system)
      || !profile.broad?.some(broad => broad.system === row.system)
      || !systems.some(system => system.id === row.system && ['sensor_radar', 'tactical_radar'].includes(system.kind))
      || (row.share_own_faction !== undefined && typeof row.share_own_faction !== 'boolean')
      || (row.receive !== undefined && typeof row.receive !== 'boolean')) throw new Error('Sharing needs a distinct actual Sensors or Tactical radar kind, own-faction intent and boolean reception.');
    shared.add(row.system);
  }
  if (profile.control != null && !closed(profile.control,['combat_lock','science_target'])) throw new Error('Sensing control bindings must be closed.');
  for (const [binding,kind] of [['combat_lock','tactical_radar'],['science_target','sensors']]) {
    const owner=profile.control?.[binding];
    if (owner != null && !systems.some(system=>system.id===owner && system.kind===kind && value.console?.some(console=>console.id===system.console))) throw new Error('Sensing control binding needs an actual owned '+kind+' System.');
    if (owner == null && profile.broad?.length && systems.some(system=>system.kind===kind)) throw new Error('Broad sensing requires explicit '+binding+' actuator ownership.');
  }
  for(const owner of [...(profile.focused || []).map(row=>row.system),profile.control?.combat_lock,profile.control?.science_target].filter(Boolean)) {
    if(!systems.find(system=>system.id===owner)?.config?.ai?.tree?.length) throw new Error("Sensing controller requires an explicit closed System AI tree.");
  }
  for (const [role, spec] of Object.entries(sensingRoles)) {
    for (const row of profile[role] || []) {
    const system = systems.find(system => system.id === row.system);
    if (!closed(row, ['system', ...spec.fields, ...(role === 'passive' ? ['mount'] : []), ...(role === 'focused' ? ['forecast'] : [])]) || !system || !(spec.kinds || [spec.kind]).includes(system.kind) || seen.has(row.system)
      || !value.console?.some(console => console.id === system.console)) throw new Error('Each instrument needs its distinct actual System kind and owning Console.');
    seen.add(row.system);
    if (!value.damage || !value.interior?.placements?.[system.id] || !value.energy?.consumer?.some(consumer => consumer.system === system.id)
      || !system.damage_component?.some(component => component.binding === 'operation')) throw new Error('Instruments require real local reserves, Operation hardware and physical rooms.');
    if (role === 'focused') {
      if (row.forecast != null && (!closed(row.forecast, ['horizon_secs']) || !positive(row.forecast.horizon_secs))) throw new Error('Focused forecast horizon must be finite and positive.');
      if (!positive(row.work_secs) || !Number.isInteger(row.history_limit) || row.history_limit < 1 || row.history_limit > 65535 || !finite(row.unfinished_retention_secs) || !value.scan)
        throw new Error('Focused sensing requires positive work/history, finite retention and actual Scan bands.');
    } else if (role === 'broad') {
      const radar = { helm_radar: value.helm_console?.radar, tactical_radar: value.weapons_console?.radar, sensor_radar: value.sensors_console?.long_range_radar }[system.kind];
      if (!radar || !positive(radar.range)) throw new Error('Broad sensing requires its actual fitted radar coverage.');
      if (!positive(row.sample_interval_secs) || !spec.fields.every(key => finite(row[key]))
        || Math.fround(row.identity_secs) > Math.fround(row.classification_secs) || Math.fround(row.classification_secs) > Math.fround(row.assessment_secs))
        throw new Error('Broad sensing requires finite cadence/errors and ordered cumulative assessment times.');
    } else {
      if (!positive(row.sample_interval_secs) || !finite(row.min_signal_strength) || !finite(row.bearing_error_rad) || Math.fround(row.bearing_error_rad) > Math.fround(Math.PI)
        || !closed(row.mount, ['facing_deg', 'arc_deg', 'range']) || typeof row.mount.facing_deg !== 'number' || !Number.isFinite(Math.fround(row.mount.facing_deg)) || !positive(row.mount.range) || !positive(row.mount.arc_deg) || Math.fround(row.mount.arc_deg) > 360
        || !system.config?.ai?.tree?.length) throw new Error('Passive sensing requires finite mount/cadence/bearing limits and an explicit closed System AI.');
    }
    }
  }
  if (systems.some(system => system.kind === 'passive_receiver' && !(profile.passive || []).some(row => row.system === system.id))) throw new Error('Every fitted passive receiver requires its matching sensing profile.');
}
export function replaceSensingSource(source, profile) {
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)].map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
  const spans = headers.filter(row => row.name === 'sensing' || row.name.startsWith('sensing.')).map(row => {
    let end = headers.find(next => next.start > row.start)?.start ?? source.length;
    const tail = source.slice(row.start, end).match(/(?:^|\n)((?:[ \t]*(?:#[^\r\n]*)?\r?\n)+)$/)?.[1]; if (tail?.includes('#')) end -= tail.length;
    return { start: row.start, end };
  });
  if (!spans.length && parse(source).sensing !== undefined) throw new Error('Expand inline sensing into tables before editing.');
  const clean = Object.fromEntries(Object.entries(profile).filter(([, row]) => row != null));
  const nl = source.includes('\r\n') ? '\r\n' : '\n', text = stringify({ sensing: clean }).trim().replace(/\n/g, nl) + nl;
  if (!spans.length) return source + (source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl) + text;
  let after = source; for (const span of spans.slice().reverse()) after = after.slice(0, span.start) + after.slice(span.end);
  return after.slice(0, spans[0].start) + text + after.slice(spans[0].start);
}
export function guardSensingReferences(value, proposed) {
  if (!value.sensing) return;
  try { validateSensing(value.sensing, proposed); } catch (error) { throw new Error('Update the affected Sensing profile first: ' + error.message); }
}
export function prepareSensingOperation(draft, dependencies, operation, schema) {
  const before = draft.read(operation.path); let source = before;
  let inspection = inspectSensing(draft, dependencies, operation.path);
  if (!operation.materialise && inspection.fields.some(field => (field.path === 'sensing' || field.path.startsWith('sensing.')) && field.owner !== operation.path)) throw new Error('Explicitly materialise inherited sensing before editing.');
  const prior = inspection.resolved.value;
  const profile = structuredClone(operation.value), candidate = { paths: () => draft.paths(), isBinary: path => draft.isBinary(path), read: path => path === operation.path ? source : draft.read(path) };
  if (operation.type === 'sensing-fit-passive') {
    const receiver = profile.passive?.find(row => row.system === operation.new_system);
    if (!receiver || inspection.systems.some(system => system.id === receiver.system)) throw new Error('Choose a fresh passive receiver System identity.');
    if (!inspection.resolved.value.energy || !inspection.resolved.value.damage || !inspection.resolved.value.interior) throw new Error('Configure actual Energy, Damage and Interiors before fitting passive equipment.');
    source = replaceSensingSource(source, profile);
    source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'system-add', id: receiver.system, kind: 'passive_receiver', console: operation.console, power_group: operation.group,
      compartment: operation.compartment, ai: initialPassiveTree(), materialise: operation.materialise }).changes[0].after;
    inspection = inspectSensing(candidate, dependencies, operation.path);
    let console = inspection.resolved.value.console.find(console => console.id === operation.console);
    if (!console) throw new Error('Choose an actual owning Console.');
    const ownedRatings = console.rating?.length ? console.rating : null, ratings = ownedRatings || inspection.resolved.value.station?.find(station => station.id === console.home_station)?.rating || [];
    if (ratings.length) source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'rating-fit-system', system: receiver.system,
      ...(ownedRatings ? { console: console.id } : { station: console.home_station }) }).changes[0].after;
    if (!parse(source).console?.some(row => row.id === console.id)) {
      if (!operation.materialise) throw new Error('Explicitly materialise the inherited receiver Console before adding its feed.');
      source += '\n' + stringify({ console: [console] }) + '\n';
    }
    source = prepareShipOperation(candidate, dependencies, schema, { path: operation.path, type: 'console-feed-add', id: console.id, source: receiver.system, kind: 'Passive' }).changes[0].after;
  }
  const remaining = new Set((profile.focused || []).map(row=>scanPublicationSource(row.system)));
  const obsolete = new Set((prior.sensing?.focused || []).map(row=>scanPublicationSource(row.system)).filter(source=>!remaining.has(source)));
  source = prunePublicationFeedsSource(source,prior,obsolete,operation.materialise,"Scan");
  validateSensing(profile, inspectSensing(candidate, dependencies, operation.path).resolved.value);
  source = replaceSensingSource(source, profile);
  parse(source); return { changes: [{ path: operation.path, before, after: source }] };
}
export async function applySensingOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-sensing-operation',
    prepare: async (captured, effective) => prepareSensingOperation(captured, effective, selected, await runtime.shipSchema()).changes,
    inspect: (candidate, effective) => inspectSensing(candidate, effective, selected.path) });
}
