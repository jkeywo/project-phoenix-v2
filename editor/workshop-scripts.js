import { parse } from 'smol-toml';
import { extractScriptUnits, inlineBlockBaseLine } from './script-editor.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

export function workshopScriptWorlds(draft) {
  return (draft?.paths() || []).filter(path => path.startsWith('assets/worlds/') && path.endsWith('.toml'))
    .flatMap(path => {
      try { return [{ path, source: draft.read(path), parsed: parse(draft.read(path)) }]; }
      catch { return []; }
    });
}

/** Start a script in a scriptless world through the same validated draft path. */
export async function createWorkshopScript({ draft, provider, runtime, worldPath, current = () => true }) {
  if (!draft) throw new Error('workshop.scripts.stale');
  const { report } = await acceptWorkshopChanges({ draft, provider, runtime, current, dependencies: null,
    stale: 'workshop.scripts.stale', refused: 'workshop.scripts.validation_refused', prepare: captured => {
      const before = captured.read(worldPath);
      if (typeof before !== 'string') throw new Error('workshop.scripts.stale');
      const parsed = parse(before);
      if (parsed.script !== undefined) throw new Error('workshop.scripts.already_present');
      const after = before + (before.endsWith('\n') ? '\n' : '\n\n')
        + "[script]\nsetup = '''\n// Add scenario callbacks here.\n'''\n";
      return [{ path: worldPath, before, after }];
    } });
  return report;
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

export async function applyWorkshopScript({ draft, provider, runtime, unit, source, current = () => true }) {
  if (!draft || !unit || typeof source !== 'string') throw new Error('workshop.scripts.stale');
  const selected = structuredClone(unit);
  const { applied } = await acceptWorkshopChanges({ draft, provider, runtime, current, dependencies: null,
    stale: 'workshop.scripts.stale', refused: 'workshop.scripts.validation_refused', prepare: captured => {
      const before = captured.read(selected.documentPath);
      const after = selected.kind === 'inline' ? replaceInlineScript(before, selected.key, source) : source;
      return before === after ? [] : [{ path: selected.documentPath, before, after }];
    } });
  return applied;
}
