/**
 * script-editor.js
 *
 * DOM-free logic for the Scenario Mode script editor (issue #983, Rhai M5).
 *
 * The scenario editor authors `.rhai` scripts (sibling files) and inline
 * `[script.*]` blocks. This module owns the pure pieces the view builds on:
 *
 *   - `tokenizeRhai(source, knownFns)` — a lightweight tokenizer for syntax
 *     highlighting (no external library; the editor loads no CDN resources).
 *   - `completionContext(textBeforeCursor)` / `matchCompletions(hostFns, ctx)` —
 *     autocomplete driven by the WASM-provided host-fn registry.
 *   - `extractScriptUnits(worldToml, worldPath)` — enumerate a world's script
 *     units (sibling file + inline blocks), mirroring the Rust loader's
 *     `lift_world_scripts`.
 *   - `siblingScriptPath(worldPath, rel)` / `inlineBlockBaseLine(rawToml, key)` —
 *     path resolution and the inline-block span mapping (the line offset a
 *     diagnostic is shifted by so it lands on the correct *document* line).
 *
 * All exports are pure and Node-testable; the DOM lives in
 * `script-editor-view.js`.
 */

/** Rhai keywords highlighted distinctly from ordinary identifiers. */
export const RHAI_KEYWORDS = new Set([
  'fn', 'let', 'const', 'if', 'else', 'switch', 'do', 'while', 'loop', 'for',
  'in', 'continue', 'break', 'return', 'throw', 'try', 'catch', 'true', 'false',
  'this', 'global', 'import', 'export', 'as', 'private', 'null',
]);

/**
 * Tokenize Rhai source into a flat array of `{ type, value }` spans whose
 * concatenated `value`s reconstruct `source` exactly (so a highlighter can
 * render them losslessly).
 *
 * Types: `comment` | `string` | `number` | `keyword` | `hostfn` | `ident` |
 * `ws` | `punct`. An identifier is tagged `hostfn` when it is in `knownFns`
 * (the host-fn registry names), so the editor can colour the scripting
 * vocabulary.
 *
 * @param {string} source
 * @param {Set<string>} [knownFns]
 * @returns {Array<{ type: string, value: string }>}
 */
export function tokenizeRhai(source, knownFns = new Set()) {
  const tokens = [];
  const src = String(source ?? '');
  const n = src.length;
  let i = 0;
  const push = (type, value) => { if (value) tokens.push({ type, value }); };

  const isWs = (c) => c === ' ' || c === '\t' || c === '\n' || c === '\r' || c === '\f' || c === '\v';
  const isDigit = (c) => c >= '0' && c <= '9';
  const isIdentStart = (c) => (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c === '_';
  const isIdentPart = (c) => isIdentStart(c) || isDigit(c);

  while (i < n) {
    const c = src[i];

    // Line comment.
    if (c === '/' && src[i + 1] === '/') {
      let j = i + 2;
      while (j < n && src[j] !== '\n') j++;
      push('comment', src.slice(i, j));
      i = j;
      continue;
    }
    // Block comment.
    if (c === '/' && src[i + 1] === '*') {
      let j = i + 2;
      while (j < n && !(src[j] === '*' && src[j + 1] === '/')) j++;
      j = Math.min(n, j + 2);
      push('comment', src.slice(i, j));
      i = j;
      continue;
    }
    // String (double or single quoted, with backslash escapes).
    if (c === '"' || c === '\'') {
      let j = i + 1;
      while (j < n) {
        if (src[j] === '\\') { j += 2; continue; }
        if (src[j] === c) { j++; break; }
        j++;
      }
      push('string', src.slice(i, j));
      i = j;
      continue;
    }
    // Whitespace.
    if (isWs(c)) {
      let j = i + 1;
      while (j < n && isWs(src[j])) j++;
      push('ws', src.slice(i, j));
      i = j;
      continue;
    }
    // Number (integers only — the scripting API is `no_float`, but tolerate a
    // stray dot rather than mis-tokenizing).
    if (isDigit(c)) {
      let j = i + 1;
      while (j < n && (isDigit(src[j]) || src[j] === '_' || src[j] === '.')) j++;
      push('number', src.slice(i, j));
      i = j;
      continue;
    }
    // Identifier / keyword / host-fn.
    if (isIdentStart(c)) {
      let j = i + 1;
      while (j < n && isIdentPart(src[j])) j++;
      const word = src.slice(i, j);
      let type = 'ident';
      if (RHAI_KEYWORDS.has(word)) type = 'keyword';
      else if (knownFns.has(word)) type = 'hostfn';
      push(type, word);
      i = j;
      continue;
    }
    // Anything else: one punctuation/operator char.
    push('punct', c);
    i += 1;
  }
  return tokens;
}

/**
 * Work out the autocomplete context at a cursor from the text before it.
 *
 * Returns `{ prefix, receiver }` where `prefix` is the identifier being typed
 * and `receiver` is:
 *   - `''`      — top-level scope (trigger builders, `on`)
 *   - `'ctx'`   — after `ctx.` (offer the `effects`/`flags`/`schedule` sub-objects)
 *   - `'effects' | 'flags' | 'schedule'` — a `ctx.<recv>.` member call
 *   - `'delay'` — after a `…)` call result, i.e. `in_seconds(n).` builder verbs
 *   - `'member'`— an unknown member context
 *
 * @param {string} textBeforeCursor
 * @returns {{ prefix: string, receiver: string }}
 */
export function completionContext(textBeforeCursor) {
  const text = String(textBeforeCursor ?? '');
  const prefixMatch = text.match(/([A-Za-z_][A-Za-z0-9_]*)$/);
  const prefix = prefixMatch ? prefixMatch[1] : '';
  const before = prefixMatch ? text.slice(0, prefixMatch.index) : text;

  if (before.endsWith('.')) {
    const head = before.slice(0, -1);
    const recvMatch = head.match(/([A-Za-z_][A-Za-z0-9_]*)$/);
    if (recvMatch) return { prefix, receiver: recvMatch[1] };
    if (/\)\s*$/.test(head)) return { prefix, receiver: 'delay' };
    return { prefix, receiver: 'member' };
  }
  return { prefix, receiver: '' };
}

/** The `ctx` sub-objects offered after `ctx.`. */
const CTX_NAMESPACES = [
  { name: 'effects', receiver: 'ctx', category: 'namespace', signature: 'effects', summary: 'Immediate effects (complete_objective, game_over, …).' },
  { name: 'flags', receiver: 'ctx', category: 'namespace', signature: 'flags', summary: 'Flag reads/writes (flags.x, flags.increment).' },
  { name: 'schedule', receiver: 'ctx', category: 'namespace', signature: 'schedule', summary: 'Deferred work (in_seconds, after).' },
];

/**
 * Filter the host-fn registry for a completion context.
 *
 * @param {Array<{name,receiver,category,signature,summary}>} hostFns
 *        The registry from `wasm_get_script_host_fns`.
 * @param {{ prefix: string, receiver: string }} ctx
 * @returns {Array<object>} matching entries, sorted by name.
 */
export function matchCompletions(hostFns, ctx) {
  const { prefix = '', receiver = '' } = ctx || {};
  const list = Array.isArray(hostFns) ? hostFns : [];
  const lower = prefix.toLowerCase();

  let pool;
  if (receiver === 'ctx') {
    pool = CTX_NAMESPACES;
  } else if (receiver === 'member') {
    pool = [];
  } else {
    pool = list.filter((h) => (h.receiver || '') === receiver);
  }

  return pool
    .filter((h) => lower === '' || h.name.toLowerCase().startsWith(lower))
    .slice()
    .sort((a, b) => a.name.localeCompare(b.name));
}

/**
 * Resolve a sibling script path relative to the world file's directory,
 * normalising to forward slashes. Mirrors the Rust loader's `sibling_path`.
 *
 * @param {string} worldPath
 * @param {string} rel
 * @returns {string}
 */
export function siblingScriptPath(worldPath, rel) {
  const relFwd = String(rel).replace(/\\/g, '/');
  const idx = Math.max(worldPath.lastIndexOf('/'), worldPath.lastIndexOf('\\'));
  if (idx < 0) return relFwd;
  return `${worldPath.slice(0, idx).replace(/\\/g, '/')}/${relFwd}`;
}

/**
 * Enumerate a world's script units, mirroring the Rust loader's
 * `lift_world_scripts`:
 *   - a top-level `script = "file.rhai"` string → one `sibling` unit
 *   - a `[script]` table of string blocks → one `inline` unit per key (sorted)
 *   - no `script` key → `[]`
 *
 * Inline units carry their `source` inline (from the parsed TOML); sibling
 * units carry only their `path` (the view loads the file). `id` is a stable
 * key for selection state.
 *
 * @param {object|null} worldToml
 * @param {string} worldPath
 * @returns {Array<{id,kind,label,path?,rel?,key?,source?}>}
 */
export function extractScriptUnits(worldToml, worldPath) {
  if (!worldToml || typeof worldToml !== 'object') return [];
  const script = worldToml.script;
  if (script == null) return [];

  if (typeof script === 'string') {
    const path = siblingScriptPath(worldPath, script);
    return [{
      id: `sibling:${path}`,
      kind: 'sibling',
      label: script,
      path,
      rel: script,
    }];
  }

  if (typeof script === 'object') {
    return Object.keys(script)
      .filter((key) => typeof script[key] === 'string')
      .sort()
      .map((key) => ({
        id: `inline:${key}`,
        kind: 'inline',
        label: `[script.${key}]`,
        key,
        source: script[key],
      }));
  }

  return [];
}

/**
 * Best-effort span mapping for an inline `[script.<key>]` block: the 0-based
 * number of lines in the raw TOML text that precede the block's *content*, to
 * be passed as the diagnostics `line_offset` when the block is shown in the
 * context of its host document.
 *
 * A multi-line `key = """` block's content starts on the line after the
 * assignment; a single-line `key = "…"` block's content is on the assignment
 * line itself. Returns `0` when the raw text is unavailable or the key is not
 * found (the block is then edited as a standalone buffer, where offset 0 is
 * already correct).
 *
 * @param {string} rawToml
 * @param {string} key
 * @returns {number}
 */
export function inlineBlockBaseLine(rawToml, key) {
  if (!rawToml || !key) return 0;
  const lines = String(rawToml).split('\n');
  const assign = new RegExp(`^\\s*(${escapeRegExp(key)}|["']${escapeRegExp(key)}["'])\\s*=`);
  let inScriptTable = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const header = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (header) {
      inScriptTable = header[1].trim() === 'script';
      continue;
    }
    if (inScriptTable && assign.test(line)) {
      // Multi-line triple-quoted block → content begins on the next line.
      if (/=\s*("""|''')\s*$/.test(line)) return i + 1;
      return i;
    }
  }
  return 0;
}

function escapeRegExp(s) {
  return String(s).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
