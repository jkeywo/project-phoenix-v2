import { parse, stringify } from 'smol-toml';
import { inspectEntityComposition } from './workshop-entity-composition.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';
const closed = (value, keys) => value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).every(key => keys.includes(key));
const factionKey = value => typeof value === 'string' ? value.replace(/^urn:uuid:/i, '').replace(/^\{(.*)\}$/, '$1').replaceAll('-', '').toLowerCase() : '';
const positive = value => Number.isSafeInteger(value) && value > 0;
const named = value => typeof value === 'string' && value.trim().length > 0;
function endpointExists(value, resource, claimed) {
  const endpoint = value.resource_services?.[resource] || { kind: 'capacity', id: resource };
  const key = endpoint.kind === 'capacity' ? 'capacity/' + endpoint.id : endpoint.kind;
  const previous = claimed.get(key);
  if (previous && previous !== resource) throw new Error('A port endpoint cannot represent two different resources.');
  const managed = value.logistics?.store?.find(store => store.endpoint?.kind === endpoint.kind && store.endpoint?.id === endpoint.id);
  if (managed && managed.resource !== resource) throw new Error("Keep the managed stock endpoint's actual resource identity.");
  claimed.set(key, resource);
  return endpoint.kind === 'torpedo_magazine'  ? resource === 'torpedoes' && !!value.torpedoes
    : endpoint.kind === 'capacity' && value.infrastructure?.capacity?.some(row => row.id === endpoint.id);
}
export function inspectPortServices(draft, dependencies, path) {
  const inspection = inspectEntityComposition(draft, dependencies, path);
  return { ...inspection, port: structuredClone(inspection.resolved.value.port_service || null) };
}
export function validatePortService(profile, value) {
  if (!closed(profile, ['policy', 'eligible_factions', 'offer']) || !['free_allied_replenishment', 'manual_offer'].includes(profile.policy)
    || !Array.isArray(profile.eligible_factions) || profile.eligible_factions.some(id => !/^[0-9a-f]{32}$/.test(factionKey(id)))
    || new Set(profile.eligible_factions.map(factionKey)).size !== profile.eligible_factions.length || (!Array.isArray(profile.offer) || !profile.offer.length))
    throw new Error('Choose a closed port policy, explicit eligible factions and finite offers. Empty factions deny all visitors.');
  const ids = new Set(), offeredResources = new Set(), claimedEndpoints = new Map();
  for (const offer of profile.offer) {
    if (!closed(offer, ['id', 'resource', 'quota', 'unit_amount', 'cost']) || !named(offer.id)
      || ids.has(offer.id) || !named(offer.resource) || offeredResources.has(offer.resource) || !Number.isSafeInteger(offer.quota) || offer.quota < 0 || !positive(offer.unit_amount)
      || !Number.isSafeInteger(offer.quota * offer.unit_amount) || (offer.cost !== undefined && !Array.isArray(offer.cost))) throw new Error('Use distinct offer IDs, positive units per lot and bounded nonnegative lot quotas.');
    ids.add(offer.id); offeredResources.add(offer.resource);
    if (!endpointExists(value, offer.resource, claimedEndpoints)) throw new Error('Create the actual offered finite stock endpoint in Resources first.');
    const resources = new Set();
    for (const cost of offer.cost || []) {
      if (!closed(cost, ['resource', 'amount']) || !named(cost.resource) || !positive(cost.amount) || resources.has(cost.resource)
        || !Number.isSafeInteger(cost.amount * offer.quota)) throw new Error('Each exchange cost requires a distinct resource and positive bounded units per lot.');
      resources.add(cost.resource);
      if (!endpointExists(value, cost.resource, claimedEndpoints)) throw new Error('Create the actual exchange-cost receiving endpoint in Resources first.');
    }
    if (profile.policy === 'free_allied_replenishment' && offer.cost?.length) throw new Error('Free allied replenishment cannot contain exchange costs.');
  }
}
export function guardPortServices(value, proposed) {
  if (value.port_service) try { validatePortService(value.port_service, proposed); }
  catch (error) { throw new Error('Reassign affected offers in Ports first: ' + error.message); }
}
export function replacePortServiceSource(source, profile) {
  const headers = [...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
    .map(match => ({ start: match.index, name: match[1].replace(/\]$/, '').trim() }));
  const spans = headers.filter(row => row.name === 'port_service' || row.name.startsWith('port_service.')).map(row => {
    let end = headers.find(next => next.start > row.start)?.start ?? source.length;
    const tail = source.slice(row.start, end).match(/(?:^|\n)((?:[ \t]*(?:#[^\r\n]*)?\r?\n)+)$/)?.[1];
    if (tail?.includes('#')) end -= tail.length;
    return { start: row.start, end };
  });
  if (!spans.length && parse(source).port_service !== undefined) throw new Error('Expand inline port services into TOML tables before editing.');
  const nl = source.includes('\r\n') ? '\r\n' : '\n', text = stringify({ port_service: profile }).trim().replace(/\n/g, nl) + nl;
  if (!spans.length) return source + (source.endsWith(nl + nl) ? '' : source.endsWith(nl) ? nl : nl + nl) + text;
  let after = source;
  for (const span of spans.slice().reverse()) after = after.slice(0, span.start) + after.slice(span.end);
  return after.slice(0, spans[0].start) + text + after.slice(spans[0].start);
}
export function preparePortServiceOperation(draft, dependencies, operation) {
  const inspection = inspectPortServices(draft, dependencies, operation.path), before = draft.read(operation.path);
  if (!operation.materialise && inspection.fields.some(row => (row.path === 'port_service' || row.path.startsWith('port_service.')) && row.owner !== operation.path))
    throw new Error('Explicitly materialise inherited port services before editing.');
  validatePortService(operation.value, inspection.resolved.value);
  return { changes: [{ path: operation.path, before, after: replacePortServiceSource(before, operation.value) }] };
}
export async function applyPortServiceOperation({ draft, provider, runtime, dependencies, operation, current = () => true }) {
  const selected = structuredClone(operation);
  return acceptWorkshopChanges({ draft, provider, runtime, dependencies, current, stale: 'stale-port-operation',
    prepare: (captured, effective) => preparePortServiceOperation(captured, effective, selected).changes,
    inspect: (candidate, effective) => inspectPortServices(candidate, effective, selected.path) });
}
