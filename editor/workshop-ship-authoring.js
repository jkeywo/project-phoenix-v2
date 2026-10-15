import { guardSecurityReferences } from './workshop-security.js';
import { guardTransitReferences } from './workshop-transit.js';
import { guardPortServiceReferences } from './workshop-ports.js';
import { guardPersonnelReferences } from './workshop-personnel.js';
import { guardWeaponGroupReferences } from './workshop-weapon-groups.js';
import { guardHelmMovementReferences } from './workshop-helm-movement.js';
import { guardCommsRadioReferences } from './workshop-comms-radio.js';
import { guardSensingReferences } from './workshop-sensing.js';
import { guardFittingReferences } from './workshop-fitting.js';
import { guardRecoveryReferences } from './workshop-resource-recovery.js';
import { guardProductionReferences } from './workshop-production.js';
import { stockPublicationSources, guardLogisticsSystemRemoval, guardLogisticsLayout } from './workshop-stock-bindings.js';
import { scanPublicationSource } from '../gui/scan-source.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
export { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { writeStateTrees, writeRepairWorkflow, rebindStateTreeChannel } from './workshop-state-trees.js';
import { writeSystemComposition, validateTurretComposition, validateIlluminationComposition, validateJammingComposition, validateProtectionComposition, validateBayComposition, validateDockMovement } from './workshop-system-composition.js';
import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
import { defaultEnergyConsumer, requiresEnergy, requireCoolantStore } from './workshop-energy-model.js';
import { replaceEnergySource } from './workshop-energy.js';
import { updateInteriorBindings } from './workshop-interiors.js';
import { defaultSystemDamage } from './workshop-damage-model.js';

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
    if (row && row._remove !== true && String(row[key]) === String(id)) return { ...block, row };
  }
  return null;
}

function tomlValue(value) {
  if (typeof value === 'string') return quote(value);
  if (typeof value === 'boolean' || (typeof value === 'number' && Number.isFinite(value))) return String(value);
  if (Array.isArray(value)) return `[${value.map(tomlValue).join(', ')}]`;
  throw new Error('unsupported-ship-value');
}

// Scalar fields belong to the record header, before either a normal nested
// table or an array table. AI definitions introduce both forms.
function recordBodyEnd(source, block) {
  const next = /^[ \t]*\[{1,2}[^\]\r\n]+\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/m.exec(source.slice(block.body, block.end));
  return next ? block.body + next.index : block.end;
}

function setKey(source, block, key, value) {
  const bodyEnd = recordBodyEnd(source, block);
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
  const bodyEnd = recordBodyEnd(source, block);
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
  const stations = value.station || [], systems = authoredAndGeneratedSystems(value), consoles = value.console || [];
  const stationIds = new Set(), systemIds = new Set();
  for (const station of stations) {
    if (['console', 'human_seeking', 'host_order', 'visiting_rating', 'auxiliary', 'command_target'].some(key => key in station)) throw finding(path, source, quote(station.id), 'Retired Station hosting fields must move to explicit Console definitions.');
    if (!ident(station.id) || station.id === 'core' || stationIds.has(station.id)) throw finding(path, source, `id = ${quote(station.id)}`, 'Station ids must be unique runtime identifiers and cannot be core.');
    stationIds.add(station.id);
  }
  const consoleIds = new Set();
  for (const console of consoles) {
    if (!ident(console.id) || consoleIds.has(console.id)) throw finding(path, source, quote(console.id), 'Console ids must be unique runtime identifiers.');
    consoleIds.add(console.id);
    const names = new Set();
    for (const rating of console.rating || []) {
      if (typeof rating.name !== 'string' || !rating.name.trim() || names.has(rating.name)) throw finding(path, source, quote(console.id), `Console ${console.id} rating names must be nonempty and unique.`);
      names.add(rating.name);
    }
    if (new Set(console.host_order || []).size !== (console.host_order || []).length) throw finding(path, source, quote(console.id), `Console ${console.id} repeats a host Station.`);
    if (!console.human_seeking && ((console.host_order || []).length || console.visiting_rating != null)) throw finding(path, source, quote(console.id), `Fixed Console ${console.id} cannot author visiting settings.`);
    const ratings = console.rating?.length ? console.rating : stations.find(station => station.id === console.home_station)?.rating || [];
    if (console.visiting_rating != null && !ratings.some(rating => rating.name === console.visiting_rating)) throw finding(path, source, quote(console.visiting_rating), `Console ${console.id} has unknown visiting rating ${console.visiting_rating}; choose an existing rating before removing its current rating.`);

    if (console.home_station != null && !stationIds.has(console.home_station)) throw finding(path, source, quote(console.id), `Console ${console.id} names an unknown home Station.`);
    for (const host of console.host_order || []) if (!stationIds.has(host)) throw finding(path, source, quote(host), `Console ${console.id} names an unknown host Station.`);
  }
  for (const console of consoles) {
    if (console.command_target != null && !stationIds.has(console.command_target)) throw finding(path, source, quote(console.command_target), `Console ${console.id} names an unknown command Station.`);
    for (const rating of console.rating || []) for (const id of [...(rating.automated_systems || []), ...(rating.detailed_systems || [])]) {
      if (!systems.some(system => system.id === id && system.console === console.id)) throw finding(path, source, quote(id), `Rating ${rating.name} may reference only Systems owned by Console ${console.id}.`);
    }
  }
  const publications = new Map(Object.entries(schema.console_publication_kinds || {}));
  publications.delete('scan');
  for (const system of systems) {
    const kind = schema.console_instance_publication_kinds?.[system.kind];
    if (kind && publications.has(system.id) && publications.get(system.id) !== kind) throw finding(path, source, quote(system.id), `System ${system.id} conflicts with a reserved publication key.`);
    if (kind && !publications.has(system.id)) publications.set(system.id, kind);
  }
  for (const source of stockPublicationSources(value, schema)) publications.set(source, schema.console_stock_publication_kind);
  for (const profile of value.sensing?.focused || []) publications.set(scanPublicationSource(profile.system),'Scan');
  for (const console of consoles) for (const feed of console.feed || []) {
    if (!publications.has(feed.source)) throw finding(path, source, quote(feed.source), `Console ${console.id} names an unknown feed source.`);
    if (!(schema.console_feed_kinds || []).includes(feed.kind)) throw finding(path, source, quote(feed.kind), `Unknown Console feed kind: ${feed.kind}`);
    if (publications.get(feed.source) !== feed.kind) throw finding(path, source, quote(feed.kind), `Console feed kind ${feed.kind} does not match publication ${feed.source} (${publications.get(feed.source)}).`);
  }
  const owner = system => consoles.find(row => row.id === system.console)?.home_station;
  for (const system of systems) {
    if (!ident(system.id) || systemIds.has(system.id)) throw finding(path, source, `id = ${quote(system.id)}`, 'System ids must be unique runtime identifiers.');
    systemIds.add(system.id);
    if (!system.generated && !schema.system_kinds.includes(system.kind)) throw finding(path, source, `kind = ${quote(system.kind)}`, `Unknown runtime System kind: ${system.kind}`);
    if (!consoleIds.has(system.console)) throw finding(path, source, quote(system.console), `System ${system.id} names an unknown Console.`);
    const console = consoles.find(row => row.id === system.console);
    if (!system.ai_only && !console.home_station && !(console.human_seeking && console.host_order?.length)) throw finding(path, source, quote(console.id), `Operable System ${system.id} needs a Console with a human hosting route.`);
    if (!system.ai_only && !console.screen?.trim()) throw finding(path, source, quote(console.id), `Operable System ${system.id} needs a Console screen.`);
  }
  for (const station of stations) for (const rating of station.rating || []) for (const reference of rating.automated_systems || []) {
    const system = systems.find(candidate => candidate.id === reference);
    if (!system || owner(system) !== station.id) throw finding(path, source, quote(reference), `Rating ${rating.name} may automate only Systems owned by Station ${station.id}.`);
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
  if (!tags.includes('ship') && !(composition.resolved.value.system || []).length) throw new Error('entity-is-not-a-ship');
  const own = parse(draft.read(path)), localDoctrine = new Set((own.behaviour?.doctrine || []).map(row => String(row.id)));
  const doctrines = (composition.resolved.value.behaviour?.doctrine || []).map(row => {
    const owner = composition.fields.find(field => field.path === `behaviour.doctrine[id=${row.id}].id`
      || field.path.startsWith(`behaviour.doctrine[id=${row.id}].`))?.owner || path;
    return { ...structuredClone(row), owner, local: localDoctrine.has(String(row.id)) };
  });
  const generated = authoredAndGeneratedSystems(composition.resolved.value).filter(system => system.generated).map(system => ({ ...system, local: false, owner: path }));
  const repairFields = composition.fields.filter(field => field.path.startsWith('repair.workflow'));
  const repair = composition.resolved.value.repair;
  return { path, repair: repair ? { ...structuredClone(repair), id: path,
    ai_inherited: repairFields.some(field => field.owner !== path),
    ai_owner: [...new Set(repairFields.map(field => field.owner))].join(', ') } : null, stations: rowsWithOwners(composition, 'station'), consoles: rowsWithOwners(composition, 'console'), systems: [...rowsWithOwners(composition, 'system'), ...generated].map(system => {
    const aiPath = system.shield_arc_id ? `shield_arc[id=${system.shield_arc_id}].ai` : `system[id=${system.id}].config.ai`;
    const fields = composition.fields.filter(field => field.path.startsWith(aiPath));
    return { ...system, energy_profile: structuredClone(composition.resolved.value.energy), ai_inherited: fields.some(field => field.owner !== path),
      ai_owner: [...new Set(fields.map(field => field.owner))].join(', ') };
  }), doctrines, composition };
}

export function prepareShipOperation(draft, dependencies, schema, operation) {
  const inspection = inspectShipAuthoring(draft, dependencies, operation.path);
  const before = draft.read(operation.path); let after = before;
  const ratingSection = operation.console ? 'console' : 'station', ratingOwner = operation.console || operation.station;
  if (operation.type === 'rating-remove') {
    const dependent = inspection.consoles.find(console => console.visiting_rating === operation.rating
      && (operation.console ? console.id === operation.console : console.home_station === operation.station && !(console.rating || []).length));
    if (dependent) throw finding(operation.path, before, quote(operation.rating), `Console ${dependent.id} uses rating ${operation.rating}; choose another visiting rating before removing it.`);
  }

  if (operation.type === 'repair-workflow-set') {
    if (!inspection.repair) throw new Error('No Repair capability on this entity.');
    after = writeRepairWorkflow(before, inspection.repair, operation.workflow, operation);
  }
  else if (operation.type === 'system-ai-set') {
    const system = inspection.systems.find(row => row.id === operation.id);
    if (!system) throw new Error('Unknown System');
    after = writeStateTrees(before, system, operation.ai, operation);
  }
  else if (operation.type === 'system-composition-set') {
    const system = inspection.systems.find(row => row.id === operation.id);
    if (!system) throw new Error('Unknown System');
    if (!system.local && !operation.materialise) throw new Error('Materialise the containing System before editing its components.');
    validateTurretComposition(system, operation.components || [], operation.channels || []);
    validateIlluminationComposition(system, operation.components || [], operation.channels || []);
    validateJammingComposition(system, operation.components || [], operation.channels || []);
    validateProtectionComposition(system, operation.components || [], operation.channels || [], inspection.composition.resolved.value);
    validateDockMovement(operation.components || []);
    validateBayComposition(system,operation.components||[],operation.channels||[]);
    const local = system.local ? before : append(before, { system: [{ id: system.id }] });
    after = writeSystemComposition(local, operation.id, operation, system);
    let rebound=system.config?.ai;
    const oldEmitter=system.component?.find(row=>['illumination_emitter','jamming_emitter'].includes(row.kind));
    const newEmitter=operation.components?.find(row=>['illumination_emitter','jamming_emitter'].includes(row.kind));
    const oldProtection=system.component?.find(row=>row.kind==='signal_protection');
    const newProtection=operation.components?.find(row=>row.kind==='signal_protection');
    const pairs=oldProtection&&newProtection ? [[oldProtection.enabled_channel,newProtection.enabled_channel]] : oldEmitter&&newEmitter
      ? [[oldEmitter.enabled_channel,newEmitter.enabled_channel],[oldEmitter.target_channel,newEmitter.target_channel]]
      : [[system.component?.find(row=>row.kind==='dock_mating')?.enabled_channel || system.component?.find(row=>row.kind==='access_boundary')?.sealed_channel,
          operation.components?.find(row=>row.kind==='dock_mating')?.enabled_channel || operation.components?.find(row=>row.kind==='access_boundary')?.sealed_channel]];
    const renamed=pairs.filter(([a,b])=>a&&b&&a!==b);
    if(rebound){
      // Rename simultaneously, so exchanging the two channel names preserves
      // each component's meaning rather than rewriting the first one twice.
      const names=new Set([...(system.channel||[]).map(row=>row.name),...(operation.channels||[]).map(row=>row.name),...renamed.flat()]);
      const temporary=renamed.map(([a,b],index)=>{let key=`workshop-rebind-${index}`;while(names.has(key))key+='-next';names.add(key);return[a,b,key];});
      for(const[a,,key]of temporary)rebound=rebindStateTreeChannel(rebound,a,key);
      for(const[,b,key]of temporary)rebound=rebindStateTreeChannel(rebound,key,b);
    }
    if(rebound&&pairs.some(([a,b])=>a&&b&&a!==b))after=writeStateTrees(after,system,rebound,{materialise:true});

  }
  else if (operation.type === 'station-add') after = append(after, { station: [{ id: operation.id, name: operation.name || operation.id,
    description: operation.description || operation.id, rank: operation.rank || '' }] });
  else if (operation.type === 'station-set' || operation.type === 'station-update') {
    const fields = operation.type === 'station-set' ? { [operation.field]: operation.value } : operation.fields;
    for (const [field, value] of Object.entries(fields)) { const block = entry(after, 'station', operation.id); if (!block) throw ownedFinding(inspection, 'station', operation.id, `Station ${operation.id} is included from another source; materialise it before editing.`);
      after = value === undefined ? removeKey(after, block, field) : setKey(after, block, field, value); }
  }
  else if (operation.type === 'station-remove') { const block = entry(after, 'station', operation.id); if (!block) throw ownedFinding(inspection, 'station', operation.id, `Station ${operation.id} is included from another source; materialise it before removing it.`); after = remove(after, block); }
  else if (operation.type === 'station-move') { const list = blocks(after, 'station', 'station'); const index = localIds(after, 'station').indexOf(operation.id);
    if (index < 0) throw ownedFinding(inspection, 'station', operation.id, `Station ${operation.id} is included from another source; materialise it before reordering it.`);
    after = move(after, list, index, operation.direction); }
  else if (operation.type === 'console-add') after = append(after, { console: [{ id: operation.id, ...operation.fields }] });
  else if (operation.type === 'console-update') {
    for (const [field, value] of Object.entries(operation.fields)) {
      const block = entry(after, 'console', operation.id);
      if (!block) throw ownedFinding(inspection, 'console', operation.id, `Console ${operation.id} is included from another source; materialise it before editing.`);
      after = value === undefined ? removeKey(after, block, field) : setKey(after, block, field, value);
    }
  } else if (operation.type === 'console-remove' || operation.type === 'console-move') {
    const block = entry(after, 'console', operation.id);
    if (!block) throw ownedFinding(inspection, 'console', operation.id, `Console ${operation.id} is included from another source; materialise it before editing.`);
    const list = blocks(after, 'console', 'console').filter(row => parse(after.slice(row.start, row.end)).console[0]._remove !== true);
    after = operation.type === 'console-remove' ? remove(after, block)
      : move(after, list, list.findIndex(row => row.start === block.start), operation.direction);
  } else if (operation.type === 'console-feed-add' || operation.type === 'console-feed-remove') {
    const block = entry(after, 'console', operation.id);
    if (!block) throw ownedFinding(inspection, 'console', operation.id, `Console ${operation.id} is included from another source; materialise it before editing.`);
    if (operation.type === 'console-feed-add') {
      const fragment = newlineOf(after) + appendNestedTable('', 'console.feed', { source: operation.source, kind: operation.kind });
      after = after.slice(0, block.end) + fragment + after.slice(block.end);
    } else {
      const relative = after.slice(block.start, block.end), feed = blocks(relative, 'console.feed', 'console')[operation.index];
      if (!feed) throw new Error('console-feed-unavailable');
      after = after.slice(0, block.start) + remove(relative, feed) + after.slice(block.end);
    }
  }
  else if (operation.type === 'system-add') {
    if (operation.kind === 'access-gate') {
      const console = inspection.consoles.find(console => console.id === operation.console);
      const ownsRepair = inspection.systems.some(system => system.id === 'repair' && system.kind === 'repair' && system.console === operation.console);
      if (!ownsRepair && !console?.feed?.some(feed => feed.source === 'repair' && feed.kind === 'Repair'))
        throw new Error('Choose the Repair Console, or add a typed Repair observation feed (source repair) to the selected Console before creating an access gate.');
    }
    const added = { id: operation.id, kind: operation.kind,
    console: operation.console, ai_only: operation.ai_only === true,
    ...(operation.power_group ? { power_group: operation.power_group } : {}),
    ...(operation.components ? { component: operation.components, channel: operation.channels || [] } : {}),
    ...(operation.ai ? { config: { ai: operation.ai } } : {}) };
    const damageValue = inspection.composition.resolved.value;
    if (damageValue.damage) added.damage_component = defaultSystemDamage(added, requiresEnergy(added)
      ? { ...damageValue, energy: { ...damageValue.energy, consumer: [...(damageValue.energy?.consumer || []), { system: added.id }] } } : damageValue);
    after = append(after, { system: [added] });
    const value = inspection.composition.resolved.value;
    if (value.energy && requiresEnergy(added)) {
      if (!operation.materialise && inspection.composition.fields.some(field => (field.path === 'energy' || field.path.startsWith('energy.')) && field.owner !== operation.path))
        throw new Error('Explicitly materialise inherited energy settings before fitting powered equipment.');
      if (!added.power_group || !(value.energy.group || []).some(group => group.id === added.power_group))
        throw new Error('Assign this machinery to an existing authored energy control group before adding it.');
      const energy = structuredClone(value.energy), coolant = structuredClone(energy.reactor.coolant || { service: 'coolant' });
      requireCoolantStore(value, coolant);
      energy.consumer ||= []; const consumer = defaultEnergyConsumer(added.id); consumer.coolant = coolant;
      energy.consumer.push(consumer); after = replaceEnergySource(after, energy);
    }
  }
  else if (operation.type === 'system-set' || operation.type === 'system-update') {
    const fields = operation.type === 'system-set' ? { [operation.field]: operation.value } : operation.fields;
    for (const [field, value] of Object.entries(fields)) { const block = entry(after, 'system', operation.id); if (!block) throw ownedFinding(inspection, 'system', operation.id, `System ${operation.id} is included from another source; materialise it before editing.`);
      after = value === undefined ? removeKey(after, block, field) : setKey(after, block, field, value); }
  }
  else if (operation.type === 'system-remove') { const block = entry(after, 'system', operation.id); if (!block) throw ownedFinding(inspection, 'system', operation.id, `System ${operation.id} is included from another source; materialise it before removing it.`); after = remove(after, block);
    const energy = inspection.composition.resolved.value.energy;
    if (energy?.consumer?.some(row => row.system === operation.id)) after = replaceEnergySource(after, { ...energy, consumer: energy.consumer.filter(row => row.system !== operation.id) });
  }
  else if (operation.type === 'system-move') { const list = blocks(after, 'system', 'system'); const index = localIds(after, 'system').indexOf(operation.id);
    if (index < 0) throw ownedFinding(inspection, 'system', operation.id, `System ${operation.id} is included from another source; materialise it before reordering it.`);
    after = move(after, list, index, operation.direction); }
  else if (operation.type === 'rating-fit-system') {
    const station = entry(after, ratingSection, ratingOwner);
    if (!station) throw ownedFinding(inspection, ratingSection, ratingOwner, 'Materialise the controlling rating source before fitting handling equipment.');
    for (const [index, row] of (station.row.rating || []).entries()) if (index > 0 || row.detailed_systems !== undefined) {
      const current = entry(after, ratingSection, ratingOwner), relative = after.slice(current.start, current.end);
      const rating = entry(relative, `${ratingSection}.rating`, row.name, 'name');
      const absolute = { ...rating, start: current.start + rating.start, body: current.start + rating.body, end: current.start + rating.end };
      const key = index === 0 ? 'detailed_systems' : 'automated_systems';
      after = setKey(after, absolute, key, [...new Set([...(row[key] || []), operation.system])]);
    }
  }
  else if (operation.type === 'rating-add') { const station = entry(after, ratingSection, ratingOwner); if (!station) throw ownedFinding(inspection, ratingSection, ratingOwner, `${ratingSection} ${ratingOwner} is included from another source; materialise it before adding a rating.`);
    const nl = newlineOf(after), fragment = `${nl}[[${ratingSection}.rating]]${nl}name = ${quote(operation.name)}${nl}automated_systems = []${nl}`;
    after = after.slice(0, station.end) + fragment + after.slice(station.end); }
  else if (operation.type === 'rating-set-systems' || operation.type === 'rating-update' || operation.type === 'rating-remove') {
    const station = entry(after, ratingSection, ratingOwner); if (!station) throw ownedFinding(inspection, ratingSection, ratingOwner, `${ratingSection} ${ratingOwner} is included from another source; materialise it before editing its ratings.`);
    const relative = after.slice(station.start, station.end), rating = entry(relative, `${ratingSection}.rating`, operation.rating, 'name');
    if (!rating) throw provenanceFinding(inspection, `${ratingSection}[id=${ratingOwner}].rating[name=${operation.rating}]`, quote(operation.rating), `Rating ${operation.rating} is included from another source; materialise it before editing.`); const absolute = { ...rating, start: station.start + rating.start, body: station.start + rating.body, end: station.start + rating.end };
    if (operation.type === 'rating-remove') after = remove(after, absolute);
    else {
      after = setKey(after, absolute, 'automated_systems', operation.systems);
      if (operation.type === 'rating-update' && operation.name !== operation.rating) {
        const updatedStation = entry(after, ratingSection, ratingOwner), updatedRelative = after.slice(updatedStation.start, updatedStation.end);
        const updatedRating = entry(updatedRelative, `${ratingSection}.rating`, operation.rating, 'name');
        after = setKey(after, { ...updatedRating, start: updatedStation.start + updatedRating.start,
          body: updatedStation.start + updatedRating.body, end: updatedStation.start + updatedRating.end }, 'name', operation.name);
      }
    }
  } else if (operation.type === 'rating-move') {
    const station = entry(after, ratingSection, ratingOwner); if (!station) throw ownedFinding(inspection, ratingSection, ratingOwner, `${ratingSection} ${ratingOwner} is included from another source; materialise it before reordering ratings.`);
    const relative = after.slice(station.start, station.end), list = blocks(relative, `${ratingSection}.rating`, ratingSection);
    const names = station.row.rating?.map(row => String(row.name)) || [], index = names.indexOf(operation.rating);
    if (index < 0) throw provenanceFinding(inspection, `${ratingSection}[id=${ratingOwner}].rating[name=${operation.rating}]`, quote(operation.rating), `Rating ${operation.rating} is included from another source; materialise it before reordering it.`);
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
  if (operation.type === 'rating-update' && operation.name !== operation.rating) {
    for (const console of inspection.consoles) {
      const usesCatalogue = operation.console ? console.id === operation.console
        : console.home_station === operation.station && !(console.rating || []).length;
      if (usesCatalogue && console.visiting_rating === operation.rating) {
        const block = entry(after, 'console', console.id);
        if (!block) throw ownedFinding(inspection, 'console', console.id, `Console ${console.id} is included from another source; materialise it before renaming its visiting rating.`);
        after = setKey(after, block, 'visiting_rating', operation.name);
      }
    }
  }
  // Composition merges keyed arrays, so duplicate local declarations must be
  // rejected before resolution can collapse them into one effective Console.
  const authoredConsoleIds = new Set();
  for (const row of parse(after).console || []) {
    if (row._remove === true) continue;
    const names = new Set();
    for (const rating of row.rating || []) {
      if (typeof rating.name !== 'string' || !rating.name.trim() || names.has(rating.name)) throw finding(operation.path, after, quote(row.id), `Console ${row.id} rating names must be nonempty and unique.`);
      names.add(rating.name);
    }
    if (!ident(row.id) || authoredConsoleIds.has(row.id)) throw finding(operation.path, after, quote(row.id), 'Console ids must be unique runtime identifiers.');
    authoredConsoleIds.add(row.id);
  }
  const candidate = { paths: () => draft.paths(), isBinary: path => draft.isBinary(path),
    read: path => path === operation.path ? after : draft.read(path) };
  if (operation.type === 'system-add') after = updateInteriorBindings(after, inspection, { add: { [operation.id]: operation.compartment }, materialise: operation.materialise === true });
  else if (operation.type === 'system-remove') after = updateInteriorBindings(after, inspection, { remove: [operation.id], materialise: operation.materialise === true });
  else if ((operation.type === 'system-set' && operation.field === 'id') || (operation.type === 'system-update' && operation.fields?.id)) {
    const id = operation.type === 'system-set' ? operation.value : operation.fields.id;
    if (id !== operation.id) after = updateInteriorBindings(after, inspection, { rename: { [operation.id]: id }, materialise: operation.materialise === true });
  }
  if (operation.type === 'system-update' && operation.compartment !== undefined
    && inspection.composition.resolved.value.interior?.placements?.[operation.id] !== operation.compartment) {
    const id = operation.fields?.id || operation.id;
    const proposed = { ...inspection, composition: inspectShipAuthoring(candidate, dependencies, operation.path).composition };
    after = updateInteriorBindings(after, proposed, { add: { [id]: operation.compartment }, materialise: operation.materialise === true });
  }
  const effectiveValue = inspectShipAuthoring(candidate, dependencies, operation.path).composition.resolved.value;
  if (operation.type === 'system-remove' || (operation.type === 'system-set' && operation.field === 'id' && operation.value !== operation.id)
    || (operation.type === 'system-update' && operation.fields?.id && operation.fields.id !== operation.id))
    guardLogisticsSystemRemoval(inspection.composition.resolved.value, operation.id);
  if (effectiveValue.interior) guardLogisticsLayout(effectiveValue, effectiveValue.interior);
  guardPersonnelReferences(inspection.composition.resolved.value, effectiveValue);
  guardSecurityReferences(inspection.composition.resolved.value, effectiveValue);
  guardPortServiceReferences(inspection.composition.resolved.value, effectiveValue);
  guardWeaponGroupReferences(inspection.composition.resolved.value, effectiveValue);
  guardHelmMovementReferences(inspection.composition.resolved.value, effectiveValue);
  guardTransitReferences(inspection.composition.resolved.value, effectiveValue);
  guardCommsRadioReferences(inspection.composition.resolved.value, effectiveValue);
  guardFittingReferences(inspection.composition.resolved.value, effectiveValue);
  guardSensingReferences(inspection.composition.resolved.value, effectiveValue);
  guardProductionReferences(inspection.composition.resolved.value, effectiveValue);
  guardRecoveryReferences(inspection.composition.resolved.value, effectiveValue);
  if (operation.type === 'system-composition-set') {
    const resolved = effectiveValue.system.find(row => row.id === operation.id);
    for (const [key, identity, desired] of [['component', 'id', operation.components], ['channel', 'name', operation.channels]]) {
      if (JSON.stringify((resolved[key] || []).map(row => row[identity])) !== JSON.stringify((desired || []).map(row => row[identity]))) {
        throw new Error('Inherited composition order is defined by its source include; edit that source to reorder it.');
      }
    }
  }
  verify(operation.path, after, schema, effectiveValue);
  return { inspection, changes: [{ path: operation.path, before, after }] };
}

export async function applyShipOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-ship-operation',
    prepare: async (captured, effective) => prepareShipOperation(captured, effective, await runtime.shipSchema(), selected).changes,
    inspect: (candidate, effective) => inspectShipAuthoring(candidate, effective, selected.path) });
}
