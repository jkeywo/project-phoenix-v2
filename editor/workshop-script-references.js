import { parse } from 'smol-toml';
import { workshopScriptUnits } from './workshop-scripts.js';
import { acceptWorkshopChanges } from './workshop-acceptance.js';

const WORLD = /^assets\/worlds\/[^/\\\r\n]+\.toml$/;
const stale = () => new Error('workshop.inspector_stale');

/** Lex only literal, single-argument load/unload calls. Comments (including
 * nested block comments), strings and computed arguments are never edit sites.
 * Template strings are deliberately left to the Scripts editor. */
export function literalWorldReferences(source) {
  const tokens = [];
  for (let i = 0; i < source.length;) {
    const start = i, c = source[i];
    if (/\s/.test(c)) { i++; continue; }
    if (source.startsWith('//', i)) {
      const end = source.indexOf('\n', i); i = end < 0 ? source.length : end; continue;
    }
    if (source.startsWith('/*', i)) {
      let depth = 1; i += 2;
      while (i < source.length && depth) {
        if (source.startsWith('/*', i)) { depth++; i += 2; }
        else if (source.startsWith('*/', i)) { depth--; i += 2; }
        else i++;
      }
      continue;
    }
    if (c === '`') return [];
    if (c === '"' || c === "'") {
      i++;
      while (i < source.length) {
        if (source[i] === '\\') { i += 2; continue; }
        if (source[i++] === c) break;
      }
      tokens.push({ type: 'string', text: source.slice(start, i), start, end: i });
    } else if (/[a-zA-Z_]/.test(c)) {
      while (i < source.length && /[a-zA-Z_0-9]/.test(source[i])) i++;
      tokens.push({ type: 'identifier', text: source.slice(start, i), start, end: i });
    } else { tokens.push({ type: 'punct', text: c, start, end: ++i }); }
  }
  return tokens.flatMap((token, i) => {
    if (token.type !== 'identifier' || !['load_world', 'unload_world'].includes(token.text)
      || tokens[i + 1]?.text !== '(' || tokens[i + 2]?.type !== 'string' || tokens[i + 3]?.text !== ')') return [];
    const literal = tokens[i + 2];
    let path;
    try { path = JSON.parse(literal.text); } catch { return []; }
    if (typeof path !== 'string') return [];
    return [{ path, kind: token.text === 'load_world' ? 'load' : 'unload',
      start: literal.start, end: literal.end, line: source.slice(0, token.start).split('\n').length }];
  });
}

/** Map decoded TOML string offsets to original source boundaries. Only the
 * selected Rhai literal is rewritten; escapes, comments and CRLF elsewhere
 * survive byte-for-byte. The ordinary parser verifies the decoder's result. */
function tomlString(source, start) {
  const quote = source[start], triple = source.startsWith(quote.repeat(3), start);
  const delimiter = quote.repeat(triple ? 3 : 1), basic = quote === '"';
  let i = start + delimiter.length, text = '';
  if (triple) {
    if (source.startsWith('\r\n', i)) i += 2;
    else if (source[i] === '\n') i++;
  }
  const boundaries = [i];
  const append = (value, end) => {
    text += value;
    for (let j = 0; j < value.length; j++) boundaries.push(end);
  };
  while (i < source.length) {
    if (source.startsWith(delimiter, i)) {
      const end = i + delimiter.length;
      let decoded;
      try { decoded = parse(`value=${source.slice(start, end)}`).value; } catch { return null; }
      return decoded === text ? { text, boundaries, start, end, basic } : null;
    }
    if (basic && source[i] === '\\') {
      const escaped = source[++i];
      if (triple && /\s/.test(escaped)) {
        while (i < source.length && /\s/.test(source[i])) i++;
        boundaries[boundaries.length - 1] = i;
        continue;
      }
      const escapes = { b: '\b', t: '\t', n: '\n', f: '\f', r: '\r', '"': '"', '\\': '\\' };
      if (Object.hasOwn(escapes, escaped)) { append(escapes[escaped], ++i); continue; }
      const count = escaped === 'u' ? 4 : escaped === 'U' ? 8 : 0;
      if (!count) return null;
      const hex = source.slice(i + 1, i + 1 + count);
      if (!new RegExp(`^[a-fA-F0-9]{${count}}$`).test(hex)) return null;
      try { append(String.fromCodePoint(parseInt(hex, 16)), i + count + 1); } catch { return null; }
      i += count + 1;
    } else { append(source[i], i + 1); i++; }
  }
  return null;
}

function inlinePayload(document, body) {
  const matches = [];
  for (let i = 0; i < document.length;) {
    if (document[i] === '#') {
      const end = document.indexOf('\n', i); i = end < 0 ? document.length : end;
    } else if (document[i] === '"' || document[i] === "'") {
      const token = tomlString(document, i);
      // Fail closed on an unsupported lexical form; do not rescan its contents.
      if (!token) return null;
      if (token.text === body) matches.push(token);
      i = token.end;
    } else i++;
  }
  return matches.length === 1 ? matches[0] : null;
}

export function scriptReferenceForms(draft, worldPath) {
  return workshopScriptUnits(draft, worldPath).flatMap(unit => {
    const document = draft.read(unit.documentPath);
    const payload = unit.kind === 'inline' ? inlinePayload(document, unit.source) : null;
    if (unit.kind === 'inline' && !payload) return [];
    return literalWorldReferences(unit.source).map(ref => ({ ...ref, unit: { ...unit, worldPath }, expectedDocument: document }));
  });
}

export async function applyScriptReference({ draft, provider, runtime, reference, path, current = () => true }) {
  if (!WORLD.test(path)) throw new Error('workshop.composition.refused.disallowed');
  return acceptWorkshopChanges({ draft, provider, runtime, current, dependencies: null,
    stale: 'workshop.inspector_stale', refused: 'workshop.scripts.validation_refused', prepare: captured => {
      const unit = reference.unit, before = captured.read(unit.documentPath);
      if (before !== reference.expectedDocument) throw stale();
      const live = scriptReferenceForms(captured, unit.worldPath).find(ref => ref.unit.documentPath === unit.documentPath
        && ref.unit.key === unit.key && ref.start === reference.start && ref.end === reference.end
        && ref.path === reference.path && ref.kind === reference.kind);
      if (!live) throw stale();
      const next = JSON.stringify(path);
      const expectedBody = unit.source.slice(0, reference.start) + next + unit.source.slice(reference.end);
      let after;
      if (unit.kind === 'inline') {
        const payload = inlinePayload(before, unit.source);
        if (!payload) throw stale();
        const encoded = payload.basic ? next.replaceAll('\\', '\\\\').replaceAll('"', '\\"') : next;
        after = before.slice(0, payload.boundaries[reference.start]) + encoded + before.slice(payload.boundaries[reference.end]);
        // This also rejects an unsupported quoting boundary without any mutation.
        if (parse(after).script?.[unit.key] !== expectedBody) throw stale();
      } else after = expectedBody;
      return before === after ? [] : [{ path: unit.documentPath, before, after }];
    } });
}
