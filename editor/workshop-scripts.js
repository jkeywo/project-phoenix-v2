import { parse } from 'smol-toml';
import { extractScriptUnits, inlineBlockBaseLine } from './script-editor.js';
import { WorkshopDocument } from './workshop-document.js';

export function workshopScriptWorlds(draft) {
  return (draft?.paths() || []).filter(path => path.startsWith('assets/worlds/') && path.endsWith('.toml'))
    .flatMap(path => {
      try { return [{ path, source: draft.read(path), parsed: parse(draft.read(path)) }]; }
      catch { return []; }
    }).filter(world => extractScriptUnits(world.parsed, world.path).length);
}

export function workshopScriptUnits(draft, worldPath) {
  const source = draft?.read(worldPath);
  if (typeof source !== 'string') return [];
  let parsed;
  try { parsed = parse(source); } catch { return []; }
  return extractScriptUnits(parsed, worldPath).map(unit => ({ ...unit,
    documentPath: unit.kind === 'sibling' ? unit.path : worldPath,
    source: unit.kind === 'sibling' ? draft.read(unit.path) : unit.source,
    lineOffset: unit.kind === 'inline' ? inlineBlockBaseLine(source, unit.key) : 0,
  })).filter(unit => typeof unit.source === 'string');
}

function quotedKey(line) {
  const match = line.match(/^\s*(?:([A-Za-z0-9_-]+)|"((?:\\.|[^"])*)"|'([^']*)')\s*=/);
  return match ? (match[1] ?? match[2] ?? match[3]) : null;
}

function encodeBasic(value, multiline) {
  let encoded = value.replaceAll('\\', '\\\\').replaceAll('"', '\\"')
    .replaceAll('\t', '\\t').replaceAll('\r', '\\r');
  if (!multiline) encoded = encoded.replaceAll('\n', '\\n');
  return encoded;
}

/** Replace only the TOML string payload for one [script] key. */
export function replaceInlineScript(source, key, next) {
  const lines = String(source).split(/(?<=\n)/);
  let offset = 0, inScript = false;
  for (const line of lines) {
    const plain = line.replace(/\r?\n$/, '');
    const header = plain.match(/^\s*\[([^\]]+)\]\s*(?:#.*)?$/);
    if (header) { inScript = header[1].trim() === 'script'; offset += line.length; continue; }
    if (!inScript || quotedKey(plain) !== key) { offset += line.length; continue; }
    const equals = plain.indexOf('=');
    const rest = plain.slice(equals + 1);
    const leading = rest.match(/^\s*/)[0].length;
    const open = offset + equals + 1 + leading;
    const quote = source.slice(open, open + 3);
    if (quote === '"""' || quote === "'''") {
      const contentStart = open + 3;
      const contentEnd = source.indexOf(quote, contentStart);
      if (contentEnd < 0) throw new Error('workshop.scripts.invalid_inline');
      if (quote === "'''" && next.includes("'''")) throw new Error('workshop.scripts.literal_delimiter');
      const leadingNewline = source.slice(contentStart, contentEnd).match(/^\r?\n/)?.[0] || '';
      const encoded = leadingNewline + (quote === '"""' ? encodeBasic(next, true) : next);
      return source.slice(0, contentStart) + encoded + source.slice(contentEnd);
    }
    const delimiter = source[open];
    if (delimiter !== '"' && delimiter !== "'") throw new Error('workshop.scripts.invalid_inline');
    let end = open + 1;
    while (end < source.length) {
      if (delimiter === '"' && source[end] === '\\') { end += 2; continue; }
      if (source[end] === delimiter) break;
      end++;
    }
    if (end >= source.length || (delimiter === "'" && next.includes("'"))) throw new Error('workshop.scripts.invalid_inline');
    const encoded = delimiter === '"' ? encodeBasic(next, false) : next;
    return source.slice(0, open + 1) + encoded + source.slice(end);
  }
  throw new Error('workshop.scripts.stale');
}

export async function applyWorkshopScript({ draft, provider, runtime, unit, source, current }) {
  if (!unit || typeof source !== 'string' || !current()) throw new Error('workshop.scripts.stale');
  const before = draft.read(unit.documentPath);
  const after = unit.kind === 'inline' ? replaceInlineScript(before, unit.key, source) : source;
  if (before === after) return false;
  const candidate = provider?.restoreDocument
    ? provider.restoreDocument(draft.snapshot()) : WorkshopDocument.restore(draft.snapshot());
  candidate.apply([{ path: unit.documentPath, before, after }]);
  const archive = provider?.save ? null : candidate.archive();
  const report = await runtime.validate(archive, candidate);
  if (!report.accepted) { const error = new Error('workshop.scripts.validation_refused'); error.report = report; throw error; }
  if (!current() || draft.read(unit.documentPath) !== before) throw new Error('workshop.scripts.stale');
  return draft.apply([{ path: unit.documentPath, before, after }]);
}
