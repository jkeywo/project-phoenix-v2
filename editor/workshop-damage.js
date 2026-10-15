import { guardSecurityReferences } from './workshop-security.js';
import { guardPortServiceReferences } from './workshop-ports.js';
import { guardPersonnelReferences } from './workshop-personnel.js';
import { guardSensingReferences } from './workshop-sensing.js';
import { guardFittingReferences } from './workshop-fitting.js';
import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
import { replaceInteriorSource } from './workshop-interiors.js';
import { inspectWeapons, prepareWeaponOperation } from './workshop-weapons.js';
import { defaultDamage, defaultSystemDamage, defaultImpact, validateDamage } from './workshop-damage-model.js';

const clone = value => structuredClone(value);
const headers = source => [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
  .map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
function append(source, value) {
  const nl = source.includes('\r\n') ? '\r\n' : '\n';
  return source + (source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl) + stringify(value).trim().replace(/\n/g, nl) + nl;
}
function removeLegacyHull(source) {
  const all = headers(source);
  for (const row of all.filter(row => row.name === 'hull' || row.name.startsWith('hull.')).reverse()) {
    let end = all.find(next => next.start > row.start)?.start ?? source.length;
    const tail = source.slice(row.start, end).match(/(?:^|\n)((?:[ \t]*(?:#[^\r\n]*)?\r?\n)+)$/)?.[1];
    if (tail?.includes('#')) end -= tail.length;
    source = source.slice(0, row.start) + source.slice(end);
  }
  return source;
}
export function replaceDamageSource(source, damage) {
  const spans = headers(source), owned = spans.filter(row => row.name === 'damage' || row.name.startsWith('damage.'));
  if (!owned.length && parse(source).damage !== undefined) throw new Error('Expand inline damage settings before editing.');
  if (!owned.length) return append(source, { damage });
  let after = source;
  for (const row of owned.slice().reverse()) {
    let end = spans.find(next => next.start > row.start)?.start ?? source.length;
    const tail = source.slice(row.start, end).match(/(?:^|\n)((?:[ \t]*(?:#[^\r\n]*)?\r?\n)+)$/)?.[1];
    if (tail?.includes('#')) end -= tail.length;
    after = after.slice(0, row.start) + after.slice(end);
  }
  const nl = source.includes('\r\n') ? '\r\n' : '\n', text = stringify({ damage }).trim().replace(/\n/g, nl) + nl;
  return after.slice(0, owned[0].start) + text + after.slice(owned[0].start);
}
export function replaceDamageRecord(source, section, row) {
  const all = headers(source), records = all.filter(header => header.name === section).map(header => ({ ...header,
    end: all.find(next => next.start > header.start && next.name !== section && !next.name.startsWith(section + '.'))?.start
      ?? all.find(next => next.start > header.start && next.name === section)?.start ?? source.length }));
  // The first subsequent root record ends this one, including repeated section arrays.
  for (const record of records) record.end = all.find(next => next.start > record.start && (next.name === section || !next.name.startsWith(section + '.')))?.start ?? source.length;
  const record = records.find(record => parse(source.slice(record.start, record.end))[section]?.[0]?.id === row.id);
  if (!record && parse(source)[section]?.some(item => item.id === row.id)) throw new Error('Expand inline equipment before editing damage components.');
  if (!record) return append(source, { [section]: [row] });
  const nl = source.includes('\r\n') ? '\r\n' : '\n';
  let body = source.slice(record.start, record.end);
  const local = parse(body)[section]?.[0];
  if (local.damage_component !== undefined && !headers(body).some(header => header.name === section + '.damage_component'))
    throw new Error('Expand inline damage component arrays before editing.');
  const nested = headers(body);
  for (const header of nested.filter(header => header.name === section + '.damage_component' || header.name.startsWith(section + '.damage_component.')).reverse()) {
    const end = nested.find(next => next.start > header.start)?.start ?? body.length;
    body = body.slice(0, header.start) + body.slice(end);
  }
  if (!row.damage_component?.length) throw new Error('Each System requires at least one damage component.');
  const serialized = stringify({ [section]: [{ id: row.id, damage_component: row.damage_component }] });
  const componentHeader = `[[${section}.damage_component]]`;
  const fragment = serialized.slice(serialized.indexOf(componentHeader)).trim().replace(/\n/g, nl);
  return source.slice(0, record.start) + body.trimEnd() + nl + fragment + nl + source.slice(record.end);
}
export function inspectDamage(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  return { ...inspection, damage: clone(inspection.resolved.value.damage || null), systems: clone(authoredAndGeneratedSystems(inspection.resolved.value)) };
}
function requireMaterialise(inspection, prefix, allowed) {
  if (!allowed && inspection.fields.some(field => (field.path === prefix || field.path.startsWith(prefix + '.') || field.path.startsWith(prefix + '[')) && field.owner !== inspection.path))
    throw new Error('Explicitly materialise inherited damage or equipment settings before editing.');
}
export function prepareDamageOperation(draft, dependencies, operation) {
  const inspection = inspectDamage(draft, dependencies, operation.path), before = draft.read(operation.path), value = clone(inspection.resolved.value);
  let after = before;
  if (operation.type === 'damage-profile') {
    requireMaterialise(inspection, 'damage', operation.materialise); value.damage = clone(operation.damage);
    after = replaceDamageSource(after, value.damage);
  } else if (operation.type === 'damage-components') {
    const system = inspection.systems.find(system => system.id === operation.system);
    if (!system) throw new Error('Choose an installed System.');
    const section = system.generated ? 'shield_arc' : 'system', id = system.generated ? system.shield_arc_id : system.id;
    const row = value[section].find(row => row.id === id);
    requireMaterialise(inspection, `${section}[id=${id}]`, operation.materialise);
    row.damage_component = clone(operation.components);
    after = replaceDamageRecord(after, section, row);
  } else if (operation.type === 'damage-create') {
    if (value.damage) throw new Error('This entity already has a damage profile.');
    if (!value.interior || !inspection.systems.length) throw new Error('Create fitted Systems and their interior before creating damage equipment.');
    value.damage = defaultDamage();
    after = replaceDamageSource(after, value.damage);
    const total = value.hull?.system_hull?.length ? value.hull.system_hull.reduce((sum, row) => sum + row.max_hp, 0)
      : value.hull?.hull_integrity ?? value.interior.compartment.reduce((sum, room) => sum + room.max_hp, 0);
    if (!Number.isFinite(total) || total <= 0) throw new Error('Author a positive structural HP budget before creating damage equipment.');
    if (inspection.fields.some(field => (field.path === 'hull' || field.path.startsWith('hull.')) && field.owner !== inspection.path))
      throw new Error('Convert the declaring inherited hull template before creating its damage profile.');
    after = removeLegacyHull(after); delete value.hull;
    requireMaterialise(inspection, 'interior', operation.materialise);
    const area = value.interior.compartment.reduce((sum, room) => sum + room.width * room.depth, 0);
    for (const room of value.interior.compartment) Object.assign(room, { max_hp: total * room.width * room.depth / area, recovery_fraction: 0.2, structural_remedy: 'structural' });
    delete value.interior.repair_placements;
    after = replaceInteriorSource(after, value.interior);
    value.infrastructure ||= {};
    value.infrastructure.capacity ||= [];
    value.resource_services ||= {};
    for (const resource of ['structural_parts', 'machinery_parts']) {
      if (!value.resource_services[resource]) {
        if (value.infrastructure.capacity.some(row => row.id === resource))
          throw new Error(`Bind the existing ${resource} capacity service before creating the profile.`);
        const stock = { id: resource, amount: resource === 'structural_parts' ? Math.ceil(total / 10) : inspection.systems.length * 3,
          ceiling: resource === 'structural_parts' ? Math.ceil(total / 10) : inspection.systems.length * 3, label: `damage.resource.${resource}` };
        value.infrastructure.capacity.push(stock);
        value.resource_services[resource] = { kind: 'capacity', id: resource };
        const nl = after.includes('\r\n') ? '\r\n' : '\n';
        const capacity = stringify({ infrastructure: { capacity: [stock] } });
        const service = stringify({ resource_services: { [resource]: value.resource_services[resource] } });
        after += nl + capacity.slice(capacity.indexOf('[[infrastructure.capacity]]')).trim().replace(/\n/g, nl) + nl
          + service.slice(service.indexOf(`[resource_services.${resource}]`)).trim().replace(/\n/g, nl) + nl;
      }
    }
    for (const system of inspection.systems) {
      const section = system.generated ? 'shield_arc' : 'system', id = system.generated ? system.shield_arc_id : system.id;
      requireMaterialise(inspection, `${section}[id=${id}]`, operation.materialise);
      const row = value[section].find(row => row.id === id); row.damage_component = defaultSystemDamage(system, value);
      after = replaceDamageRecord(after, section, row);
    }
    after = after.replace(/^[ \t]*(?:hull_max_hp|hull_damaged_threshold_pct|hull_disabled_threshold_pct|hull_debuff_magnitude)[ \t]*=[^\r\n]*(?:\r?\n|$)/gm, '');
    const candidate = { paths: () => draft.paths(), isBinary: path => draft.isBinary(path), read: path => path === operation.path ? after : draft.read(path) };
    for (const row of inspectWeapons(candidate, dependencies, operation.path).weapons) {
      if (!row.weapon.impact) {
        row.weapon.impact = defaultImpact(row.weapon.delivery.kind);
        after = prepareWeaponOperation(candidate, dependencies, { path: operation.path, section: row.section, originalId: row.weapon.id,
          weapon: row.weapon, materialise: operation.materialise === true }).changes[0].after;
      }
    }
    // Explicit creation authors real finite stocks; later values are editable
    // through Resources. No runtime fallback or second inventory is introduced.
  } else throw new Error('Unsupported damage authoring operation.');
  guardFittingReferences(inspection.resolved.value, inspectDamage({ paths: () => draft.paths(), isBinary: path => draft.isBinary(path), read: path => path === operation.path ? after : draft.read(path) }, dependencies, operation.path).resolved.value);
  guardSensingReferences(inspection.resolved.value, inspectDamage({ paths: () => draft.paths(), isBinary: path => draft.isBinary(path), read: path => path === operation.path ? after : draft.read(path) }, dependencies, operation.path).resolved.value);
  guardPortServiceReferences(inspection.resolved.value, value);
  guardPersonnelReferences(inspection.resolved.value, value);
  guardSecurityReferences(inspection.resolved.value, value);
  validateDamage(value.damage, authoredAndGeneratedSystems(value), value);
  parse(after);
  return { changes: [{ path: operation.path, before, after }] };
}
export async function applyDamageOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = clone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-damage-operation',
    prepare: (captured, effective) => prepareDamageOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectDamage(candidate, effective, selected.path) });
}
