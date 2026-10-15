import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const object = value => value && typeof value === 'object' && !Array.isArray(value);
const closed = (value, keys) => object(value) && Object.keys(value).every(key => keys.includes(key));
const finite = value => typeof value === 'number' && Number.isFinite(Math.fround(value));
export const LOCAL_SECURITY_ACTIONS = Object.freeze(['secure_contain', 'assist_evacuation']);
export const SECURITY_PRIORITIES = Object.freeze(['life_safety', 'evacuation_underway', 'urgent_objective', 'threat_containment', 'optional']);

export function validateSecurity(profile, entity) {
  if (!closed(profile, ['team_count', 'deploy_duration_secs', 'withdraw_duration_secs', 'range', 'local'])
    || !Number.isInteger(profile.team_count) || profile.team_count < 1 || profile.team_count > 255
    || !finite(profile.deploy_duration_secs) || profile.deploy_duration_secs < 0
    || !finite(profile.withdraw_duration_secs) || profile.withdraw_duration_secs < 0
    || !finite(profile.range) || profile.range <= 0) throw new Error('Security requires its accepted finite team count, crossing terms and reach.');
  const systems = authoredAndGeneratedSystems(entity);
  if (!systems.some(system => system.id === 'security' && system.kind === 'security')) throw new Error('Security requires its actual controlling System.');
  if (profile.local === false || profile.local == null) return;
  const local = profile.local, rooms = new Set((entity.interior?.compartment || []).map(room => room.id));
  if (!closed(local, ['team_homes', 'target']) || !Array.isArray(local.team_homes) || local.team_homes.length !== profile.team_count
    || local.team_homes.some(home => !rooms.has(home)) || !Array.isArray(local.target) || !local.target.length
    || !Array.isArray(entity.energy?.consumer) || !entity.energy.consumer.some(consumer => consumer.system === 'security' && finite(consumer.continuous_cost_per_sec) && consumer.continuous_cost_per_sec > 0)) throw new Error('Local Security requires actual team homes, compartment work and positive Security work energy.');
  const targets = new Set();
  for (const target of local.target) {
    if (!closed(target, ['compartment', 'action']) || !rooms.has(target.compartment) || targets.has(target.compartment)
      || !Array.isArray(target.action) || !target.action.length) throw new Error('Local Security requires distinct actual compartment work.');
    targets.add(target.compartment); const actions = new Set();
    for (const action of target.action) {
      if (!closed(action, ['action', 'duration_secs', 'risk', 'priority', 'outcome_flag', 'outcome_value', 'warning'])
        || !LOCAL_SECURITY_ACTIONS.includes(action.action) || actions.has(action.action)
        || !finite(action.duration_secs) || action.duration_secs <= 0 || !finite(action.risk) || action.risk < 0 || action.risk > 1
        || !SECURITY_PRIORITIES.includes(action.priority)
        || (action.outcome_flag != null && (typeof action.outcome_flag !== 'string' || !action.outcome_flag.trim()))
        || (action.outcome_value != null && !Number.isSafeInteger(action.outcome_value))
        || (action.warning != null && typeof action.warning !== 'string')) throw new Error('Local Security action terms are unsupported or nonfinite.');
      actions.add(action.action);
    }
  }
}
export function inspectSecurity(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  return { ...inspection, security: structuredClone(inspection.resolved.value.security || null),
    rooms: structuredClone(inspection.resolved.value.interior?.compartment || []),
    inherited: inspection.fields.some(row => (row.path === 'security' || row.path.startsWith('security.')) && row.owner !== path) };
}
export function guardSecurityReferences(before, after) {
  if (before.security?.local && before.security.local !== false) validateSecurity(after.security, after);
}
const stable = value => JSON.stringify((function order(row) { return Array.isArray(row) ? row.map(order) : object(row) ? Object.fromEntries(Object.keys(row).sort().map(key => [key, order(row[key])])) : row; })(value));
function replaceSecurity(source, security) {
  const before = parse(source), newline = source.includes('\r\n') ? '\r\n' : '\n';
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)];
  const spans = headers.map((match, index) => ({ name: match[1].replace(/\]$/, '').trim(), start: match.index, end: headers[index + 1]?.index ?? source.length }));
  for (const span of spans.filter(row => row.name === 'security' || row.name.startsWith('security.')).reverse()) source = source.slice(0, span.start) + source.slice(span.end);
  source += (source.endsWith('\n') ? newline : newline + newline) + stringify({ security }).replace(/\r?\n/g, newline);
  const after = parse(source), without = value => { const copy = structuredClone(value); delete copy.security; return stable(copy); };
  if (stable(after.security) !== stable(security) || without(before) !== without(after)) throw new Error('Expand Security into ordinary tables before editing; unrelated source must remain unchanged.');
  return source;
}
export function prepareSecurityOperation(draft, dependencies, operation) {
  const inspection = inspectSecurity(draft, dependencies, operation.path);
  if (inspection.inherited && !operation.materialise) throw new Error('Explicitly materialise inherited Security terms before editing.');
  validateSecurity(operation.value, inspection.resolved.value);
  const before = draft.read(operation.path), after = replaceSecurity(before, operation.value);
  return { changes: before === after ? [] : [{ path: operation.path, before, after }] };
}
export async function applySecurityOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current,
    prepare: (captured, effective) => prepareSecurityOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectSecurity(candidate, effective, selected.path) });
}
