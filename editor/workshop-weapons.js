import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const clone = value => structuredClone(value);
const families = { beam: 'weapons_console.phaser_banks', bolt: 'weapons_console.blaster_banks', guided: 'torpedoes.tubes' };
const beamNames = { auto_arc_deg: 'auto_arc_deg', damage_per_sec: 'beam_damage_per_sec', duration_secs: 'beam_duration_secs', cooldown_secs: 'cooldown_secs', cycle_jitter: 'cycle_jitter', color: 'beam_color', shield_pierce: 'shield_pierce' };
export const WEAPON_PARAMETERS = Object.freeze({
  beam: ['auto_arc_deg', 'damage_per_sec', 'duration_secs', 'cooldown_secs', 'cycle_jitter', 'shield_pierce'],
  bolt: ['volley_count', 'volley_interval_secs', 'cooldown_secs', 'charge_time_secs', 'projectile_speed', 'collision_radius', 'visual_scale', 'damage', 'shield_pierce', 'recoil_impulse', 'screenshake_magnitude'],
  guided: ['load_time', 'volley_max', 'ai_target_count'],
});
export function defaultWeapon(kind = 'beam') {
  const delivery = {
    beam: { kind, auto_arc_deg: 90, damage_per_sec: 10, duration_secs: 1, cooldown_secs: 1, cycle_jitter: 0, shield_pierce: 0 },
    bolt: { kind, volley_count: 1, volley_interval_secs: 0.1, cooldown_secs: 1, charge_time_secs: 0, projectile_speed: 30, collision_radius: 0.2, visual_scale: 1, damage: 10, shield_pierce: 0, recoil_impulse: 0, screenshake_magnitude: 0 },
    guided: { kind, volley_max: 1 },
  }[kind];
  if (!delivery) throw new Error('Unsupported weapon delivery.');
  return { id: '', mount: { facing_deg: 0, fire_arc_deg: 90, range: 40 }, delivery: clone(delivery), ai: { idle: true } };
}
// Match TorpedoConfig's supported defaults when the author leaves global
// tuning implicit. Generic mount validation compares against these values.
export const guidedWeaponRange = value => (value.torpedoes?.speed ?? 15) * (value.torpedoes?.lifespan ?? 20);
const at = (object, path) => path.split('.').reduce((value, key) => value?.[key], object);
export function inspectWeapons(draft, dependencies, path) {
  const composition = inspectEntityComposition(draft, dependencies, path), value = composition.resolved.value;
  const weapons = (value.weapon || []).map(row => ({ section: 'weapon', weapon: clone(row) }));
  for (const [kind, section] of Object.entries(families)) for (const row of at(value, section) || []) {
    const delivery = { kind };
    for (const field of [...WEAPON_PARAMETERS[kind], ...(kind === 'beam' ? ['color'] : [])]) {
      const name = kind === 'beam' ? beamNames[field] : field;
      if (row[name] !== undefined) delivery[field] = clone(row[name]);
    }
    weapons.push({ section, weapon: { id: row.id, mount: { facing_deg: row.facing_deg ?? 0, fire_arc_deg: row.fire_arc_deg,
      range: kind === 'beam' ? row.beam_range : kind === 'bolt' ? row.range : guidedWeaponRange(value) },
    delivery, ...Object.fromEntries(['marker', 'barrels', 'pattern', 'ai'].filter(key => row[key] !== undefined).map(key => [key, clone(row[key])])) } });
  }
  return { ...composition, weapons };
}
function spans(source, section) {
  const headers = [...source.matchAll(/^[ \t]*\[(\[?)([^\r\n]+?)\]\]?[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, name: match[2].trim(), array: match[1] === '[' }));
  return headers.filter(header => header.name === section && header.array).map(header => ({ ...header,
    end: headers.find(next => next.start > header.start && (next.name === section || !next.name.startsWith(section + '.')))?.start ?? source.length }));
}
function fragment(section, row) {
  return stringify(section.split('.').reverse().reduce((value, key) => ({ [key]: value }), [row])).trim();
}
export function prepareWeaponOperation(draft, dependencies, operation) {
  const inspection = inspectWeapons(draft, dependencies, operation.path), before = draft.read(operation.path);
  const selected = inspection.weapons.find(row => row.section === operation.section && row.weapon.id === operation.originalId);
  if (operation.originalId !== undefined && !selected) throw new Error('Selected weapon no longer exists.');
  const weapon = clone(operation.weapon), section = selected?.section || 'weapon';
  if (!weapon?.id?.trim() || !families[weapon.delivery?.kind]) throw new Error('Choose a weapon id and supported delivery.');
  if (selected && weapon.id !== selected.weapon.id) throw new Error('Create another weapon to use a different id.');
  if (!selected && inspection.weapons.some(row => row.weapon.id === weapon.id)) throw new Error('Weapon id already exists.');
  if (selected && weapon.delivery.kind !== selected.weapon.delivery.kind) throw new Error('Create another weapon to use a different delivery.');
  for (const key of ['facing_deg', 'fire_arc_deg', 'range']) if (!Number.isFinite(weapon.mount?.[key])) throw new Error('Mount settings must be finite numbers.');
  const prefix = `${section}[id=${weapon.id}]`;
  if (inspection.fields.some(field => (field.path === prefix || field.path.startsWith(prefix + '.')) && field.owner !== inspection.path)
    && !operation.materialise) throw new Error('Explicitly materialise inherited weapon settings before editing.');
  let row = weapon;
  if (section !== 'weapon') {
    row = clone((at(inspection.resolved.value, section) || []).find(item => item.id === weapon.id));
    row.facing_deg = weapon.mount.facing_deg; row.fire_arc_deg = weapon.mount.fire_arc_deg;
    if (weapon.delivery.kind === 'beam') row.beam_range = weapon.mount.range;
    else if (weapon.delivery.kind === 'bolt') row.range = weapon.mount.range;
    else if (weapon.mount.range !== selected.weapon.mount.range) throw new Error('Guided range is set by shared torpedo speed and lifespan.');
    for (const [key, value] of Object.entries(weapon.delivery)) if (key !== 'kind') row[weapon.delivery.kind === 'beam' ? beamNames[key] : key] = value;
    for (const key of ['marker', 'barrels', 'pattern']) { delete row[key]; if (weapon[key] !== undefined) row[key] = clone(weapon[key]); }
  }
  const localRows = at(parse(before), section), records = spans(before, section);
  const record = records.find(item => (at(parse(before.slice(item.start, item.end)), section) || []).some(entry => entry.id === weapon.id));
  if (!record && localRows?.some(item => item.id === weapon.id)) throw new Error('Expand inline weapon definitions into TOML tables before editing.');
  const nl = before.includes('\r\n') ? '\r\n' : '\n', text = fragment(section, row).replace(/\n/g, nl) + nl;
  const after = record ? before.slice(0, record.start) + text + before.slice(record.end)
    : before + (before.endsWith(nl + nl) ? '' : before.endsWith(nl) ? nl : nl + nl) + text;
  parse(after);
  return { changes: [{ path: operation.path, before, after }] };
}
export async function applyWeaponOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = clone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-weapon-operation',
    prepare: (captured, effective) => prepareWeaponOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectWeapons(candidate, effective, selected.path) });
}
