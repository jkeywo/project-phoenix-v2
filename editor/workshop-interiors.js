import { guardTransitReferences } from './workshop-transit.js';
import { guardPortServiceReferences } from './workshop-ports.js';
import { guardPersonnelReferences } from './workshop-personnel.js';
import { guardSensingReferences } from './workshop-sensing.js';
import { guardFittingReferences } from './workshop-fitting.js';
import { guardRecoveryReferences } from './workshop-resource-recovery.js';
import { guardProductionReferences } from './workshop-production.js';
import { guardLogisticsLayout } from './workshop-stock-bindings.js';
import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const clone = value => structuredClone(value);
export const initialAccessGateTree = (channel = 'sealed') => ({ root: 'access-gate', tree: [{ id: 'access-gate', initial: 'follow-order', state: [{ id: 'follow-order',
  task: [{ id: 'respect-seal', action: { kind: 'write', channel, value: { source: 'fact', name: 'seal_requested' } } }] }] }] });
export function inspectInterior(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  return { ...inspection, interior: clone(inspection.resolved.value.interior || null),
    systems: clone(authoredAndGeneratedSystems(inspection.resolved.value)),
    teamCount: inspection.resolved.value.repair?.repair_team_count || 0 };
}
/** An explicit starter, never a production parser fallback. */
export function initialInterior(systems, teamCount = 0) {
  return { team_homes: Array.from({ length: teamCount }, () => 'bridge'),
    deck: [{ id: 'main', label: 'interior.deck.main.label', damage_weight: 1 }],
    compartment: [{ id: 'bridge', deck: 'main', label: 'interior.room.bridge.label', x: -5, z: -5, width: 10, depth: 10, material: 'bulkhead',
      max_hp: 100, recovery_fraction: 0.2, structural_remedy: 'structural' }],
    connection: [], placements: Object.fromEntries(systems.map(system => [system.id, 'bridge'])) };
}
export function replaceInteriorSource(source, interior) {
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
  const spans = headers.filter(row => row.name === 'interior' || row.name.startsWith('interior.'))
    .map(row => {
      const next = headers.find(next => next.start > row.start); let end = next?.start ?? source.length;
      if (!next || (next.name !== 'interior' && !next.name.startsWith('interior.'))) {
        const tail = source.slice(row.start, end).match(/(?:^|\n)((?:[ \t]*(?:#[^\r\n]*)?\r?\n)+)$/)?.[1];
        if (tail?.includes('#')) end -= tail.length;
      }
      return { start: row.start, end };
    });
  if (!spans.length && parse(source).interior !== undefined) throw new Error('Expand inline interior definitions into TOML tables before editing.');
  const nl = source.includes('\r\n') ? '\r\n' : '\n';
  const text = stringify({ interior }).trim().replace(/\n/g, nl) + nl;
  if (!spans.length) return source + (source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl) + text;
  let after = source;
  for (const span of spans.slice().reverse()) after = after.slice(0, span.start) + after.slice(span.end);
  return after.slice(0, spans[0].start) + text + after.slice(spans[0].start);
}
/** Cheap form feedback. Accepted content still passes the real runtime validator. */
export function validateInterior(interior, systems, teamCount) {
  const id = value => typeof value === 'string' && /^[a-z0-9-]+$/.test(value);
  if (!interior || !Array.isArray(interior.deck) || !interior.deck.length || !Array.isArray(interior.compartment)
    || !interior.compartment.length || !Array.isArray(interior.connection) || !interior.placements
    || !Array.isArray(interior.team_homes)) throw new Error('Interior requires decks, compartments, connections, placements and team homes.');
  const unique = (rows, kind) => {
    const seen = new Set();
    for (const row of rows) { if (!id(row.id) || seen.has(row.id)) throw new Error(`Invalid or duplicate ${kind} id: ${row.id}`); seen.add(row.id); }
    return seen;
  };
  const decks = unique(interior.deck, 'deck'), rooms = unique(interior.compartment, 'compartment');
  unique(interior.connection, 'connection');
  for (const deck of interior.deck) if (typeof deck.label !== 'string' || !deck.label.trim() || !Number.isFinite(deck.damage_weight) || deck.damage_weight <= 0)
    throw new Error('Deck damage weights must be finite and positive.');
  for (const room of interior.compartment) if (!decks.has(room.deck) || !id(room.material) || typeof room.label !== 'string' || !room.label.trim()
    || ['x', 'z', 'width', 'depth', 'max_hp', 'recovery_fraction'].some(key => !Number.isFinite(room[key])) || room.width <= 0 || room.depth <= 0
    || room.max_hp <= 0 || room.recovery_fraction <= 0 || room.recovery_fraction >= 1 || !id(room.structural_remedy))
    throw new Error('Compartments require a known deck/material, positive finite dimensions/HP, recovery fraction in (0,1) and a structural remedy.');
  for (let i = 0; i < interior.compartment.length; i++) for (const b of interior.compartment.slice(i + 1)) {
    const a = interior.compartment[i];
    if (a.deck === b.deck && Math.min(a.x + a.width, b.x + b.width) > Math.max(a.x, b.x)
      && Math.min(a.z + a.depth, b.z + b.depth) > Math.max(a.z, b.z)) throw new Error('Physical compartments on one deck cannot overlap.');
  }
  const known = new Set(systems.map(system => system.id));
  const boundaries = new Set(systems.filter(system => system.kind === 'access-gate').map(system => system.id));
  for (const connection of interior.connection) if (!rooms.has(connection.from) || !rooms.has(connection.to)
    || connection.from === connection.to || !Number.isFinite(connection.travel_secs) || connection.travel_secs <= 0
    || (connection.boundary !== undefined && !boundaries.has(connection.boundary)))
    throw new Error('Connections require distinct known compartments, positive travel time and an access-gate boundary System.');
  if (Object.keys(interior.placements).length !== known.size || [...known].some(system => !rooms.has(interior.placements[system]))
    || Object.keys(interior.placements).some(system => !known.has(system)))
    throw new Error('Place every resolved and generated System in exactly one known compartment.');
  if (interior.repair_placements !== undefined) throw new Error('Convert retired repair placements to compartment structure (increment 11).');
  if (interior.team_homes.length !== teamCount || interior.team_homes.some(home => !rooms.has(home)))
    throw new Error('Choose one known home compartment for every repair team.');
  const reached = new Set([interior.compartment[0].id]);
  for (let changed = true; changed;) {
    changed = false;
    for (const edge of interior.connection) if (reached.has(edge.from) || reached.has(edge.to)) {
      for (const endpoint of [edge.from, edge.to]) if (!reached.has(endpoint)) { reached.add(endpoint); changed = true; }
    }
  }
  if (reached.size !== rooms.size) throw new Error('Connect every compartment, including connections between decks.');
}
export function prepareInteriorOperation(draft, dependencies, operation) {
  const inspection = inspectInterior(draft, dependencies, operation.path), before = draft.read(operation.path);
  if (inspection.fields.some(field => (field.path === 'interior' || field.path.startsWith('interior.')) && field.owner !== inspection.path)
    && !operation.materialise) throw new Error('Explicitly materialise inherited interior settings before editing.');
  const interior = clone(operation.interior);
  const teamCount = operation.teamCount ?? inspection.teamCount;
  if (!Number.isSafeInteger(teamCount) || teamCount < 0) throw new Error('Choose a non-negative whole repair team count.');
  validateInterior(interior, inspection.systems, teamCount);
  guardPortServiceReferences(inspection.resolved.value, { ...inspection.resolved.value, interior });
  guardPersonnelReferences(inspection.resolved.value, { ...inspection.resolved.value, interior });
  guardFittingReferences(inspection.resolved.value, { ...inspection.resolved.value, interior });
  guardSensingReferences(inspection.resolved.value, { ...inspection.resolved.value, interior });
  guardLogisticsLayout(inspection.resolved.value, interior);
  guardProductionReferences(inspection.resolved.value, { ...inspection.resolved.value, interior });
  guardRecoveryReferences(inspection.resolved.value, { ...inspection.resolved.value, interior });
  guardTransitReferences(inspection.resolved.value, { ...inspection.resolved.value, interior });
  let after = replaceInteriorSource(before, interior);
  if (teamCount !== inspection.teamCount) {
    if (!inspection.resolved.value.repair) throw new Error('Configure the Repair capability before changing its team count.');
    if (inspection.fields.some(field => field.path === 'repair.repair_team_count' && field.owner !== inspection.path) && !operation.materialise)
      throw new Error('Explicitly materialise the inherited repair team count before changing it.');
    const nl = before.includes('\r\n') ? '\r\n' : '\n', header = /^[ \t]*\[repair\][ \t]*(?:#.*)?(?:\r?\n|$)/m.exec(after);
    if (!header) after += `${nl}[repair]${nl}repair_team_count = ${teamCount}${nl}`;
    else {
      const rest = after.slice(header.index + header[0].length), end = /^[ \t]*\[/m.exec(rest)?.index ?? rest.length;
      const body = rest.slice(0, end), replaced = /^[ \t]*repair_team_count[ \t]*=.*$/m.test(body)
        ? body.replace(/^([ \t]*repair_team_count[ \t]*=)[^\r\n]*/m, `$1 ${teamCount}`) : `repair_team_count = ${teamCount}${nl}` + body;
      after = after.slice(0, header.index + header[0].length) + replaced + rest.slice(end);
    }
  }
  parse(after);
  return { changes: [{ path: operation.path, before, after }] };
}
/** Other hardware forms keep their placement changes in their own transaction. */
export function updateInteriorBindings(source, inspection, { add = {}, remove = [], rename = {}, materialise = false } = {}) {
  const value = inspection.resolved?.value || inspection.composition?.resolved.value, interior = clone(value.interior);
  if (!interior || (!Object.keys(add).length && !remove.length && !Object.keys(rename).length)) return source;
  const fields = inspection.fields || inspection.composition?.fields || [];
  if (fields.some(field => (field.path === 'interior' || field.path.startsWith('interior.')) && field.owner !== inspection.path) && !materialise)
    throw new Error('Explicitly materialise inherited interior settings before changing fitted equipment.');
  for (const id of remove) {
    if (interior.connection.some(edge => edge.boundary === id)) throw new Error('Reassign boundary connections in Interiors before removing this System.');
    delete interior.placements[id];
  }
  for (const [oldId, newId] of Object.entries(rename)) {
    if (interior.placements[oldId] !== undefined) { interior.placements[newId] = interior.placements[oldId]; delete interior.placements[oldId]; }
    for (const edge of interior.connection) if (edge.boundary === oldId) edge.boundary = newId;
  }
  for (const [id, compartment] of Object.entries(add)) {
    if (!interior.compartment.some(room => room.id === compartment)) throw new Error('Choose an existing physical compartment in Interiors before adding equipment.');
    interior.placements[id] = compartment;
  }
  return replaceInteriorSource(source, interior);
}
export async function applyInteriorOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = clone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-interior-operation',
    prepare: (captured, effective) => prepareInteriorOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectInterior(candidate, effective, selected.path) });
}
