import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const object = value => value && typeof value === 'object' && !Array.isArray(value);
const closed = (value, keys) => object(value) && Object.keys(value).every(key => keys.includes(key));
const id = value => typeof value === 'string' && /^[a-z0-9-]+$/.test(value);
const label = value => typeof value === 'string' && value.trim();
const competencies = value => Array.isArray(value) && value.length > 0 && value.every(item => ['mechanical_repair','medical_care'].includes(item)) && new Set(value).size === value.length;
export function validatePersonnel(profile, entity) {
  if (!closed(profile, ['repair_team', 'officer', 'care']) || !Array.isArray(profile.repair_team) || !Array.isArray(profile.officer)) throw new Error('Personnel requires closed team and officer lists.');
  const count = entity.repair?.repair_team_count, homes = entity.interior?.team_homes;
  if (!Number.isInteger(count) || count <= 0 || !Array.isArray(homes) || homes.length !== count || !entity.damage
    || !authoredAndGeneratedSystems(entity).some(system => system.id === 'repair' && system.kind === 'repair')) throw new Error('Configure actual finite Repair teams, their home compartments and physical remedy machinery first.');
  const rooms = new Set((entity.interior?.compartment || []).map(room => room.id)), ids = new Set(), slots = new Set();
  for (const team of profile.repair_team) {
    if (!closed(team, ['id', 'label', 'team_idx', 'competencies']) || !id(team.id) || !label(team.label) || ids.has(team.id)
      || !Number.isInteger(team.team_idx) || team.team_idx < 0 || team.team_idx >= count || slots.has(team.team_idx) || !competencies(team.competencies)) throw new Error('Bind each named team to a distinct actual Repair team with mechanical repair competency.');
    ids.add(team.id); slots.add(team.team_idx);
  }
  ids.clear();
  for (const officer of profile.officer) {
    if (!closed(officer, ['id', 'label', 'home_compartment', 'competencies', 'repair_work_multiplier', 'initial_condition']) || !id(officer.id) || !label(officer.label)
      || !['fit','minor','severe'].includes(officer.initial_condition ?? 'fit') || ids.has(officer.id) || !id(officer.home_compartment) || !rooms.has(officer.home_compartment) || !competencies(officer.competencies)
      || typeof officer.repair_work_multiplier !== 'number' || !Number.isFinite(Math.fround(officer.repair_work_multiplier)) || Math.fround(officer.repair_work_multiplier) < 1
      || !profile.repair_team.some(team => homes[team.team_idx] === officer.home_compartment && team.competencies.some(item => officer.competencies.includes(item)))) throw new Error('Each finite officer needs a compatible actual team at its home and a finite work multiplier of at least one.');
    ids.add(officer.id);
  }
  const care = profile.care;
  if (care !== undefined && care !== false) {
    if (!closed(care, ['system','minor_work_secs','severe_work_secs','requirement']) || care.system !== 'repair'
      || ![care.minor_work_secs, care.severe_work_secs].every(value => typeof value === 'number' && Number.isFinite(Math.fround(value)) && Math.fround(value) > 0)
      || !Array.isArray(care.requirement) || care.requirement.length === 0
      || !profile.repair_team.some(team => team.competencies.includes('medical_care'))
      || !entity.interior?.placements?.[care.system]
      || !entity.energy?.consumer?.some(consumer => consumer.system === care.system && consumer.continuous_cost_per_sec > 0)) throw new Error('Care needs actual paid Repair machinery, a physical service site and an existing MedicalCare team.');
    const resources = new Set();
    for (const part of care.requirement) {
      const endpoint = entity.resource_services?.[part.service];
      const capacity = entity.infrastructure?.capacity?.find(row => endpoint?.kind === 'capacity' && row.id === endpoint.id);
      if (!closed(part,['resource','service','amount']) || !label(part.resource) || !label(part.service) || !Number.isSafeInteger(part.amount) || part.amount <= 0
        || resources.has(part.resource) || !capacity || (capacity.ceiling ?? capacity.amount) < part.amount
        || !entity.logistics?.store?.some(store => store.role === 'local' && store.resource === part.resource && store.system === care.system && store.source
          && store.endpoint?.kind === 'capacity' && store.endpoint.id === endpoint.id && store.compartment === entity.interior.placements[care.system])) throw new Error('Medication needs unique positive requirements, real finite stock and a managed local replenishment source.');
      resources.add(part.resource);
    }
  }
}
export function inspectPersonnel(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  const personnel = structuredClone(inspection.resolved.value.personnel || { repair_team: [], officer: [] });
  return { ...inspection, personnel, inherited: inspection.fields.some(row => (row.path === 'personnel' || row.path.startsWith('personnel.')) && row.owner !== path) };
}
export function guardPersonnelReferences(before, after) {
  if (before.personnel) validatePersonnel(after.personnel, after);
}
const stable = value => JSON.stringify((function order(row) { return Array.isArray(row) ? row.map(order) : object(row) ? Object.fromEntries(Object.keys(row).sort().map(key => [key, order(row[key])])) : row; })(value));
function writePersonnel(source, value) {
  const before = parse(source), newline = source.includes('\r\n') ? '\r\n' : '\n';
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)];
  const spans = headers.map((match, index) => ({ name: match[1].replace(/\]$/, '').trim(), start: match.index, end: headers[index + 1]?.index ?? source.length }));
  for (const span of spans.filter(row => row.name === 'personnel' || row.name.startsWith('personnel.')).reverse()) source = source.slice(0, span.start) + source.slice(span.end);
  source += (source.endsWith('\n') ? newline : newline + newline) + stringify({ personnel: value }).replace(/\r?\n/g, newline);
  const after = parse(source), without = value => { const copy = structuredClone(value); delete copy.personnel; return stable(copy); };
  if (stable(after.personnel) !== stable(value) || without(before) !== without(after)) throw new Error('Expand personnel into ordinary tables before editing; unrelated source must remain unchanged.');
  return source;
}
export function preparePersonnelOperation(draft, dependencies, operation) {
  const inspection = inspectPersonnel(draft, dependencies, operation.path);
  if (inspection.inherited && !operation.materialise) throw new Error('Explicitly materialise the inherited personnel roster before editing.');
  validatePersonnel(operation.value, inspection.resolved.value);
  const before = draft.read(operation.path), after = writePersonnel(before, operation.value);
  return { changes: before === after ? [] : [{ path: operation.path, before, after }] };
}
export async function applyPersonnelOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current,
    prepare: (captured, effective) => preparePersonnelOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectPersonnel(candidate, effective, selected.path) });
}
