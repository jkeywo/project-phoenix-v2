import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';
import { parse, stringify } from 'smol-toml';
/** Closed stock publications exist only for actual configured local owners. */
export function stockPublicationSources(value, schema) {
  if (schema?.console_stock_publication_kind !== 'Stock') return [];
  const systems = new Set(authoredAndGeneratedSystems(value).map(row => row.id));
  return [...new Set((value.logistics?.store || []).filter(row => row.role === 'local' && systems.has(row.system))
    .map(row => 'stock/' + row.system))].sort();
}
export function requiredStockFeeds(value, schema) {
  const sources = new Set(stockPublicationSources(value, schema)), feeds = [], seen = new Set();
  const systems = new Map(authoredAndGeneratedSystems(value).map(row => [row.id, row]));
  const inventory = systems.get(value.logistics?.handling?.[0]?.system)?.console;
  for (const store of value.logistics?.store || []) {
    const source = 'stock/' + store.system;
    if (store.role !== 'local' || !sources.has(source)) continue;
    for (const console of [systems.get(store.system)?.console, inventory].filter(Boolean)) {
      const key = JSON.stringify([console, source]);
      if (!seen.has(key)) { feeds.push({ console, source, kind: 'Stock' }); seen.add(key); }
    }
  }
  return feeds;
}
/** Remove only publications that cease to exist, in the profile's transaction. */
export function pruneStockFeedsSource(source, value, obsolete, materialise) {
  return prunePublicationFeedsSource(source,value,obsolete,materialise,"Stock");
}
/** Existing typed-feed source cleanup, shared by actual Stock and Scan owners. */
export function prunePublicationFeedsSource(source, value, obsolete, materialise, kind) {
  if (!obsolete.size) return source;
  const nl = source.includes('\r\n') ? '\r\n' : '\n';
  for (const console of value.console || []) {
    const removed = feed => feed.kind === kind && obsolete.has(feed.source);
    if (!console.feed?.some(removed)) continue;
    const local = parse(source).console?.find(row => row.id === console.id);
    if (!local) {
      if (!materialise) throw new Error('Explicitly materialise inherited Consoles before removing their obsolete typed feeds.');
      source += nl + stringify({ console: [{ ...console, feed: console.feed.filter(feed => !removed(feed)) }] }).trim().replace(/\n/g, nl) + nl;
      continue;
    }
    const headers = [...source.matchAll(/^[ \t]*(\[{1,2})([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)]
      .map(match => ({ name: match[2].replace(/\]$/, '').trim(), start: match.index }));
    const start = headers.findIndex((header, i) => header.name === 'console'
      && parse(source.slice(header.start, headers.slice(i + 1).find(next => !next.name.startsWith('console.'))?.start ?? source.length)).console?.[0]?.id === console.id);
    if (start < 0) throw new Error('Expand inline Consoles and feeds into TOML tables before editing typed publication bindings.');
    const end = headers.findIndex((header, i) => i > start && !header.name.startsWith('console.'));
    const feeds = headers.slice(start + 1, end < 0 ? undefined : end).filter(header => header.name === 'console.feed');
    if (feeds.length !== (local.feed || []).length) throw new Error('Expand inline Console feeds into TOML tables before editing stock bindings.');
    const ranges = feeds.flatMap((header, i) => removed(local.feed[i]) ? [{ start: header.start,
      end: headers[headers.indexOf(header) + 1]?.start ?? source.length }] : []);
    for (const range of ranges.reverse()) source = source.slice(0, range.start) + source.slice(range.end);
  }
  return source;
}
export function guardLogisticsSystemRemoval(value, id) {
  if (value.logistics?.handling?.some(row => row.system === id) || value.logistics?.store?.some(row => row.system === id))
    throw new Error('Reassign this System\'s managed stores and handling in Logistics before removing or renaming it.');
}
export function guardLogisticsLayout(value, interior) {
  const rooms = new Set((interior.compartment || []).map(row => row.id));
  if ((value.logistics?.storage || []).some(row => !rooms.has(row.compartment))
    || (value.logistics?.store || []).some(row => !rooms.has(row.compartment)
      || (row.role === 'local' && interior.placements?.[row.system] !== row.compartment)))
    throw new Error('Reassign managed stock locations in Logistics before removing a compartment or moving its local owner.');
}
export function guardManagedStockBounds(value, endpoint, amount, ceiling) {
  const store = value.logistics?.store?.find(row => row.endpoint.kind === endpoint.kind
    && (endpoint.kind !== 'capacity' || row.endpoint.id === endpoint.id));
  if (!store) return;
  if (store.target > ceiling || store.reserve > ceiling)
    throw new Error('Reduce the managed store target/protected floor in Logistics before shrinking its capacity.');
  const bay = value.logistics.storage.find(row => row.compartment === store.compartment);
  if (!bay) return;
  const occupancy = value.logistics.store.filter(row => row.compartment === store.compartment).reduce((total, row) => {
    const level = row.id === store.id ? amount : row.endpoint.kind === 'torpedo_magazine'
      ? (value.torpedoes.initial_count ?? value.torpedoes.count ?? 10)
      : value.infrastructure.capacity.find(capacity => capacity.id === row.endpoint.id)?.amount || 0;
    return total + level * (bay.unit_space[row.resource] || 0);
  }, 0);
  if (!Number.isSafeInteger(occupancy) || occupancy > bay.space)
    throw new Error('Increase compatible compartment storage in Logistics before adding this initial stock.');
}
