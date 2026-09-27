import { parse } from 'smol-toml';
import { WorkshopDocument } from './workshop-document.js';

const quote = value => JSON.stringify(String(value));
const nlOf = text => text.includes('\r\n') ? '\r\n' : '\n';

export function slotWorlds(draft) {
  return (draft?.paths() || []).filter(path => path.startsWith('assets/worlds/') && path.endsWith('.toml')
    && typeof draft.read(path) === 'string');
}

export function inspectSlots(draft, path) {
  const world = parse(draft.read(path));
  return (world.ship_slot || []).map(slot => ({ id: slot.id, label: slot.label || '',
    ships: (slot.ships || []).map(ship => ship.template_path), default_ship: slot.default_ship,
    unclaimed: slot.unclaimed || 'backfill' }));
}

function tableHeaders(source) {
  const headers = [];
  let mode = 'normal', offset = 0;
  for (const line of source.split(/(?<=\n)/)) {
    if (mode === 'normal') {
      const header = line.trimEnd().match(/^[ \t]*(?:\[\[([^\]\r\n]+)\]\]|\[([^\]\r\n]+)\])(?:[ \t]*(?:#.*)?)?$/);
      if (header) headers.push({ name: header[1] || header[2], start: offset });
    }
    for (let i = 0; i < line.length; i += 1) {
      const three = line.slice(i, i + 3), char = line[i];
      if (mode === 'normal') {
        if (char === '#') break;
        if (three === '"""') { mode = 'multi-basic'; i += 2; }
        else if (three === "'''") { mode = 'multi-literal'; i += 2; }
        else if (char === '"') mode = 'basic';
        else if (char === "'") mode = 'literal';
      } else if (mode === 'basic') {
        if (char === '\\') i += 1;
        else if (char === '"') mode = 'normal';
      } else if (mode === 'literal') {
        if (char === "'") mode = 'normal';
      } else if (mode === 'multi-basic') {
        if (char === '\\') i += 1;
        else if (three === '"""') { mode = 'normal'; i += 2; }
      } else if (three === "'''") { mode = 'normal'; i += 2; }
    }
    offset += line.length;
  }
  return headers;
}

function slotRanges(source) {
  const headers = tableHeaders(source);
  return headers.filter(row => row.name === 'ship_slot').map(row => ({ start: row.start,
    end: headers.find(next => next.start > row.start && next.name !== 'ship_slot.ships')?.start ?? source.length }));
}

function renderSlot(row, nl, priorOffers = []) {
  const lines = ['[[ship_slot]]', `id = ${quote(row.id)}`];
  if (row.label) lines.push(`label = ${quote(row.label)}`);
  lines.push(`default_ship = ${quote(row.default_ship)}`, `unclaimed = ${quote(row.unclaimed)}`);
  for (const path of row.ships) {
    lines.push('', '[[ship_slot.ships]]', `template_path = ${quote(path)}`);
    const prior = priorOffers.find(offer => offer.template_path === path);
    if (prior?.label) lines.push(`label = ${quote(prior.label)}`);
  }
  return lines.join(nl) + nl + nl;
}

export function prepareSlotOperation(draft, path, operation) {
  const before = draft.read(path);
  if (typeof before !== 'string') throw new Error('workshop.slot.error_world');
  const parsed = parse(before);
  const existing = inspectSlots(draft, path), ranges = slotRanges(before), nl = nlOf(before);
  if (ranges.length !== existing.length) throw new Error('workshop.slot.error_source');
  const index = existing.findIndex(row => row.id === operation.target);
  if (operation.type !== 'add' && index < 0) throw new Error('workshop.slot.error_select');
  if (operation.type === 'add' && existing.some(row => row.id === operation.row?.id)) throw new Error('workshop.slot.error_duplicate');
  const row = operation.row;
  if (operation.type !== 'remove') {
    if (!row || !/^[A-Za-z0-9][A-Za-z0-9_-]*$/.test(row.id || '')) throw new Error('workshop.slot.error_id');
    if (!Array.isArray(row.ships) || !row.ships.length || new Set(row.ships).size !== row.ships.length
      || row.ships.some(value => !value || !draft.paths().includes(value))) throw new Error('workshop.slot.error_ships');
    if (!row.ships.includes(row.default_ship)) throw new Error('workshop.slot.error_default');
    if (!['backfill', 'absent'].includes(row.unclaimed)) throw new Error('workshop.slot.error_policy');
    if (existing.some((item, i) => i !== index && item.id === row.id)) throw new Error('workshop.slot.error_duplicate');
  }
  let after;
  if (operation.type === 'add') after = before + (before.endsWith(nl) ? nl : nl + nl) + renderSlot(row, nl);
  else if (operation.type === 'update') after = before.slice(0, ranges[index].start)
    + renderSlot(row, nl, parsed.ship_slot[index].ships) + before.slice(ranges[index].end);
  else if (operation.type === 'remove') after = before.slice(0, ranges[index].start) + before.slice(ranges[index].end);
  else throw new Error('workshop.slot.error_operation');
  return { path, before, after };
}

export async function applySlotOperation({ draft, provider, runtime, path, operation, current = () => true }) {
  const revision = draft.sourceRevision, change = prepareSlotOperation(draft, path, operation);
  const candidate = provider?.restoreDocument ? provider.restoreDocument(draft.snapshot()) : WorkshopDocument.restore(draft.snapshot());
  candidate.apply([change]);
  const report = await runtime.validate(candidate.kind === 'mod' && !provider?.save ? candidate.archive() : null, candidate);
  if (!report?.accepted) { const error = new Error('workshop.slot.error_runtime'); error.report = report; throw error; }
  if (!current() || draft.sourceRevision !== revision || draft.read(path) !== change.before) throw new Error('workshop.slot.error_stale');
  draft.apply([change]);
  return report;
}
