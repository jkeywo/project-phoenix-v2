import { t, wireText } from './strings.js';

const object = value => value && typeof value === 'object' && !Array.isArray(value);
const finite = value => typeof value === 'number' && Number.isFinite(Math.fround(value));
const stamp = value => Number.isSafeInteger(value) && value >= 0;
const fraction = value => finite(value) && value >= -1 && value <= 1;

/** Only the already-permitted Helm board and delivered observations enter. */
export function buildHelmMovement(value, { systemIds = [], controlSources = {}, tracks = [], tick } = {}) {
  if (!object(value)) return null;
  const axis = (role, row) => {
    if (!object(row) || typeof row.system !== 'string' || !row.system || !object(row.intent)
      || !['assisted', 'direct'].includes(row.intent.mode)) return null;
    const intent = row.intent;
    if (intent.mode === 'direct' && !fraction(intent[role === 'steering' ? 'rate' : 'force'])) return null;
    if (intent.mode === 'assisted' && role === 'lateral' && !fraction(intent.value)) return null;
    if (intent.mode === 'assisted' && role !== 'lateral') {
      const demand = intent.demand;
      if (!object(demand)) return null;
      if (role === 'steering') {
        if (!(demand.kind === 'rate' && fraction(demand.value))
          && !(demand.kind === 'heading' && finite(demand.heading_rad) && demand.heading_rad >= 0 && demand.heading_rad < Math.PI * 2)) return null;
      } else if (!(demand.kind === 'fraction' && fraction(demand.value))
        && !(['absolute', 'match'].includes(demand.kind) && finite(demand.speed))) return null;
      if (demand.kind === 'match' && (demand.speed < 0 || typeof demand.contact !== 'string' || !demand.contact
        || !stamp(demand.observed_tick) || (stamp(tick) && demand.observed_tick > tick))) return null;
    }
    return { role, system_id: row.system, intent: structuredClone(intent), mode: intent.mode,
      available: row.available === true,
      controllable: systemIds.includes(row.system) && controlSources[row.system] === 'Human' };
  };
  const thrust = axis('thrust', value.thrust), steering = axis('steering', value.steering);
  const lateral = value.lateral == null ? null : axis('lateral', value.lateral);
  if (!thrust || !steering || (value.lateral != null && !lateral)
    || new Set([thrust.system_id, steering.system_id, ...(lateral ? [lateral.system_id] : [])]).size !== (lateral ? 3 : 2)) return null;
  const candidates = new Map(), conflicts = new Set();
  for (const record of Array.isArray(tracks) ? tracks : []) {
    if (typeof record?.contact_id !== 'string' || !record.contact_id || record.predicted !== false
      || record.selectable !== true || !stamp(record.observed_tick) || !stamp(tick) || record.observed_tick > tick
      || !Array.isArray(record.velocity) || record.velocity.length !== 2 || !record.velocity.every(finite)) continue;
    const speed = Math.hypot(...record.velocity);
    if (!finite(speed)) continue;
    const row = { contact: record.contact_id, name: record.identity?.name ? wireText(record.identity.name) : t('console.sensors.basic_contact'),
      speed, observed_tick: record.observed_tick, age: tick - record.observed_tick };
    const previous = candidates.get(row.contact);
    if (previous && (previous.speed !== row.speed || previous.observed_tick !== row.observed_tick)) conflicts.add(row.contact);
    candidates.set(row.contact, row);
  }
  const demand = thrust.intent.demand;
  return { axes: [thrust, steering, ...(lateral ? [lateral] : [])],
    contacts: [...candidates.values()].filter(row => !conflicts.has(row.contact)).sort((a, b) => a.contact.localeCompare(b.contact)),
    match: demand?.kind === 'match' ? { contact: demand.contact, speed: demand.speed,
      observed_tick: demand.observed_tick, age: stamp(tick) ? tick - demand.observed_tick : null,
      observation_available: value.match_observation_available === true } : null };
}
