import { parse, stringify } from 'smol-toml';

/** Replace only the selected System's composition, as one accepted transaction.
 * Nested identities are scoped to that System; inherited data requires an
 * explicit materialisation action, never a write into its supplying include. */
export function writeSystemComposition(source, id, operation, effective) {
  const headers = [...source.matchAll(/^[ \t]*\[(\[?)([^\]\r\n]+)\]\]?[^\r\n]*(?:\r?\n|$)/gm)]
    .map(match => ({ name: match[2].trim(), array: match[1] === '[', start: match.index, end: match.index + match[0].length }));
  const start = headers.findIndex((header, index) => {
    if (header.name !== 'system' || !header.array) return false;
    const end = headers.slice(index + 1).find(next => next.name === 'system' || !next.name.startsWith('system.'))?.start ?? source.length;
    return parse(source.slice(header.start, end)).system?.[0]?.id === id;
  });
  if (start < 0) throw new Error('Materialise the containing System before editing its components.');
  const first = headers[start];
  const end = headers.slice(start + 1).find(next => next.name === 'system' || !next.name.startsWith('system.'))?.start ?? source.length;
  const local = parse(source.slice(first.start, end)).system[0];
  const components = structuredClone(operation.components || []), channels = structuredClone(operation.channels || []);
  for (const [key, identity, next] of [['component', 'id', components], ['channel', 'name', channels]]) {
    const inherited = (effective[key] || []).filter(row => !(local[key] || []).some(own => own[identity] === row[identity]
      && !own._remove && JSON.stringify(own) === JSON.stringify(row)));
    if (inherited.length && !operation.materialise) throw new Error('Materialise inherited components and channels explicitly before editing this composition.');
    // Reconciled inherited entries retain their include order. Do not promise
    // an order that the runtime merge will silently ignore.
    const kept = inherited.filter(row => next.some(item => item[identity] === row[identity]));
    if (kept.map(row => row[identity]).join('\0') !== next.filter(row => kept.some(item => item[identity] === row[identity])).map(row => row[identity]).join('\0')) {
      throw new Error('Inherited composition order is defined by its source include.');
    }
    for (const row of inherited) if (!next.some(item => item[identity] === row[identity])) next.push({ [identity]: row[identity], _remove: true });
  }
  let body = source.slice(first.start, end);
  const remove = headers.filter(header => header.start > first.start && header.start < end
    && ['system.component', 'system.channel'].includes(header.name));
  for (const header of remove.reverse()) {
    const stop = headers.find(next => next.start > header.start && !next.name.startsWith(`${header.name}.`))?.start ?? source.length;
    body = body.slice(0, header.start - first.start) + body.slice(Math.min(stop, end) - first.start);
  }
  const nl = source.includes('\r\n') ? '\r\n' : '\n';
  const serialized = stringify({ system: [{ component: components, channel: channels }] })
    .replace(/^\[\[system\]\]\r?\n/, '').trim().replace(/\n/g, nl);
  return source.slice(0, first.start) + body.trimEnd() + nl + nl + serialized + nl + nl + source.slice(end);
}
