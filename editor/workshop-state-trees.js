import { parse, stringify } from 'smol-toml';
import { stripRemovals } from './entity-includes.js';

const clone = value => structuredClone(value);
const stable = value => JSON.stringify(value, (_, item) => item && !Array.isArray(item) && typeof item === 'object' ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item);
const mergeNamed = (base = [], local = [], combine = (_, row) => clone(row)) => {
  const result = clone(base);
  for (const row of local) { const at = result.findIndex(item => item.id === row.id);
    if (at < 0) result.push(clone(row)); else result[at] = combine(result[at], row); }
  return result;
};
/** Read-only resolved display. Execution and authoritative validation stay Rust-owned. */
export function resolveStateTree(ai, id, stack = []) {
  if (stack.includes(id)) throw new Error(`Tree inheritance cycle: ${[...stack, id].join(' -> ')}`);
  const local = ai.tree?.find(tree => tree.id === id);
  if (!local) throw new Error(`Unknown tree: ${id}`);
  const base = local.extends ? resolveStateTree(ai, local.extends, [...stack, id]) : {};
  const resolved = { ...clone(base), ...clone(local),
    parameters: { ...base.parameters, ...local.parameters }, memory: { ...base.memory, ...local.memory },
    state: mergeNamed(base.state, local.state, (parent, state) => ({ ...parent, ...clone(state),
      task: mergeNamed(parent.task, state.task), transition: mergeNamed(parent.transition, state.transition) })) };
  delete resolved.extends;
  return resolved;
}

/** Replace the selected System's AI subtree, preserving its other authored bytes. */
export function writeStateTrees(source, system, ai, { materialise = false } = {}) {
  const section = system.shield_arc_id ? 'shield_arc' : 'system';
  const recordId = system.shield_arc_id || system.id;
  const aiPath = section === 'shield_arc' ? 'shield_arc.ai' : 'system.config.ai';
  const readAi = row => section === 'shield_arc' ? row?.ai : row?.config?.ai;
  const nl = source.includes('\r\n') ? '\r\n' : '\n';
  const headers = [...source.matchAll(/^[ \t]*\[(\[?)([^\]\r\n]+)\]\]?[^\r\n]*(?:\r?\n|$)/gm)]
    .map(match => ({ name: match[2].trim(), array: match[1] === '[', start: match.index, end: match.index + match[0].length }));
  let found;
  for (let i = 0; i < headers.length; i += 1) {
    const header = headers[i]; if (header.name !== section || !header.array) continue;
    const end = headers.slice(i + 1).find(next => next.name === section || !next.name.startsWith(section + '.'))?.start ?? source.length;
    const local = parse(source.slice(header.start, end))[section]?.[0];
    if (local?.id === recordId && local._remove !== true) { found = { start: header.start, end, local }; break; }
  }
  if ((!found || stable(stripRemovals(readAi(found.local))) !== stable(system.config?.ai)) && !materialise) {
    throw new Error('Explicitly materialise inherited AI before editing it.');
  }
  // Keep explicit resets on every subsequent edit: dropping them would resurrect
  // inherited fields or nodes after a previously materialised library was edited.
  const resetIds = new Set([...(system.config?.ai?.tree || []).map(tree => tree.id),
    ...(readAi(found?.local)?.tree || []).filter(tree => tree._remove).map(tree => tree.id)]);
  const authored = { ...clone(ai), tree: [
    ...[...resetIds].map(id => ({ id, _remove: true })), ...clone(ai.tree || []),
  ] };
  const record = section === 'shield_arc' ? { ai: authored } : { config: { ai: authored } };
  const serialized = stringify({ [section]: [record] })
    .replace(new RegExp(`^\\[\\[${section}\\]\\]\\r?\\n`), '')
    .replace(/^\s*\[system.config\]\r?\n/, '').trim().replace(/\n/g, nl);
  if (!found) return source.trimEnd() + nl + nl + `[[${section}]]${nl}id = ${JSON.stringify(recordId)}${nl}` + serialized + nl;
  const first = headers.find(header => header.start > found.start && header.start < found.end && header.name === aiPath);
  if (!first && readAi(found.local)) throw new Error('Expand inline AI into TOML tables before visual editing.');
  if (first) {
    const end = headers.find(header => header.start > first.start && !header.name.startsWith(aiPath + '.'))?.start ?? source.length;
    return source.slice(0, first.start) + serialized + nl + nl + source.slice(Math.min(end, found.end));
  }
  return source.slice(0, found.end).trimEnd() + nl + nl + serialized + nl + nl + source.slice(found.end);
}

/** Rename only actual channel references, never parameter/memory names or facts. */
export function rebindStateTreeChannel(ai, before, after) {
  const next = clone(ai);
  const visit = value => {
    if (!value || typeof value !== 'object') return;
    if ((value.kind === 'write' || value.kind === 'write_number' || value.source === 'pending') && value.channel === before) value.channel = after;
    for (const child of Object.values(value)) if (typeof child === 'object') visit(child);
  };
  visit(next); return next;
}
export const idleStateTree = () => ({ root: 'idle', tree: [{ id: 'idle', initial: 'idle',
  parameters: {}, memory: {}, state: [{ id: 'idle', task: [], transition: [] }] }] });

/** The same library format in the independent Repair Job context. */
export function writeRepairWorkflow(source, repair, workflow, { materialise = false } = {}) {
  const local = parse(source).repair?.workflow;
  if (stable(stripRemovals(local)) !== stable(repair?.workflow) && !materialise)
    throw new Error('Explicitly materialise inherited repair workflow before editing it.');
  const ids = new Set([...(repair?.workflow?.tree || []).map(row => row.id),
    ...(local?.tree || []).filter(row => row._remove).map(row => row.id)]);
  const authored = { ...clone(workflow), tree: [...[...ids].map(id => ({ id, _remove: true })), ...clone(workflow.tree || [])] };
  const nl = source.includes('\r\n') ? '\r\n' : '\n';
  const serialized = stringify({ repair: { workflow: authored } }).replace(/^\s*\[repair\]\r?\n/, '').trim().replace(/\n/g, nl);
  const headers = [...source.matchAll(/^[ \t]*\[\[?([^\]\r\n]+)\]\]?[^\r\n]*(?:\r?\n|$)/gm)];
  const first = headers.find(row => row[1].trim() === 'repair.workflow');
  if (!first && local) throw new Error('Expand inline repair.workflow into TOML tables before visual editing.');
  if (!first) return source.trimEnd() + nl + nl + serialized + nl;
  const end = headers.find(row => row.index > first.index && !row[1].trim().startsWith('repair.workflow.'))?.index ?? source.length;
  return source.slice(0, first.index) + serialized + nl + nl + source.slice(end);
}
