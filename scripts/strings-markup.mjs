/**
 * scripts/strings-markup.mjs — finding un-localised display text in MARKUP.
 *
 * `strings-literals.mjs` scans `.textContent = …` assignments. That is one of
 * three ways English reaches a screen in this client, and it was the only one
 * the gate could see (issue #976). The other two live in markup:
 *
 *   - a web component's shadow-DOM template, built as a template literal in the
 *     constructor — `<span class="bar-label">Station</span>`;
 *   - a text-bearing HTML attribute — `<ph-station-damage label="Core">`.
 *
 * Neither is a `textContent` assignment, so both rendered hardcoded English in
 * every locale with `check-strings --strict` green. AGENTS.md rule 11 is only
 * real if something checks it, so this module checks it.
 *
 * Two scans, two different predicates, because the two positions carry
 * different amounts of ambiguity:
 *
 *   - A TEXT NODE is unambiguous. Nothing but displayed characters can appear
 *     between `>` and `<`, so any word there is display text. `isMarkupText`
 *     is therefore deliberately loose — it asks only "are there letters?".
 *   - An ATTRIBUTE is ambiguous, which is the whole reason this was filed as a
 *     decision rather than a patch: `title="close"` may be a tooltip or a DOM
 *     hook. `isDisplayAttrValue` resolves it on capitalisation and spacing —
 *     see the rule there, and the blind spot it deliberately accepts.
 *
 * The allowlist is explicit and stays explicit. A denylist would silently
 * enrol every future `data-*` hook and drive someone to a blanket suppression,
 * which is worse than no rule at all.
 *
 * One rule governs everything below: **never fail silent**. Zero findings must
 * mean "I read this and found nothing", never "I stopped reading". Where the JS
 * lexer loses its place it emits an `unscannable` finding naming the spot, and
 * `--strict` fails on that the same way it fails on untranslated English.
 */

/**
 * Attributes whose value is read by a player.
 *
 * `data-screen-label` is this codebase's own: the console shell shows it as the
 * screen's name, and every site already pairs it with `data-i18n-attr`, so
 * listing it pins that convention rather than introducing work.
 *
 * `value` is deliberately ABSENT. It is genuinely dual-purpose — `<option
 * value="fore">` is a wire token, `<input type="submit" value="Send">` is
 * display text — and nothing in this client uses the display half.
 */
export const TEXT_BEARING_ATTRS = new Set([
  'alt',
  'aria-label',
  'data-screen-label',
  'label',
  'placeholder',
  'title',
]);

/** HTML elements with no closing tag, so they must not be pushed on the stack. */
const VOID_ELEMENTS = new Set([
  'area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input',
  'link', 'meta', 'param', 'source', 'track', 'wbr',
]);

/** Elements whose body is code rather than markup. */
const RAW_TEXT_ELEMENTS = new Set(['script', 'style']);

/**
 * A single all-lowercase token, or lowercase tokens joined by `-`, `_`, `.` or
 * `/`: `close`, `nav-back`, `camera_fore`, `console.repair.title`, `assets/a`.
 */
const MACHINE_TOKEN = /^[a-z0-9]+(?:[-_./][a-z0-9]+)*$/;

/** Two consecutive letters — the shortest thing that can be a word ("AU"). */
const HAS_WORD = /[A-Za-z]{2,}/;

// ── Text extraction helpers ─────────────────────────────────────────────────

/**
 * Remove `${…}` interpolations, counting braces so an object literal inside one
 * does not close it early. What remains is the LITERAL text of the template —
 * the only part an author could have failed to localise, since a `${t('id')}`
 * is by definition already resolved.
 *
 * @param {string} s
 * @returns {string}
 */
export function stripInterpolations(s) {
  let out = '';
  let i = 0;
  while (i < s.length) {
    if (s[i] === '$' && s[i + 1] === '{') {
      let depth = 1;
      i += 2;
      while (i < s.length && depth > 0) {
        if (s[i] === '{') depth += 1;
        else if (s[i] === '}') depth -= 1;
        i += 1;
      }
      continue;
    }
    out += s[i];
    i += 1;
  }
  return out;
}

/**
 * Replace HTML entities with a space. `&hellip;` and `&#8592;` are punctuation
 * that carries no locale; `&amp;` would otherwise read as the word "amp" and
 * report every ampersand in the client as untranslated English.
 *
 * @param {string} s
 */
const stripEntities = (s) => s.replace(/&(?:#\d+|#x[0-9a-fA-F]+|[a-zA-Z]+);/g, ' ');

/**
 * Whether a text node holds display text.
 *
 * Loose on purpose: a text node cannot hold a css class, an element id or a
 * wire token, so "does it contain letters" is the whole question. Glyph-only
 * runs (`▲`, `—`, `·`, `%`, `°`) have no letters and pass through silently,
 * which is right — they read the same in every locale.
 *
 * @param {string} text
 * @returns {boolean}
 */
export function isMarkupText(text) {
  return HAS_WORD.test(stripEntities(stripInterpolations(text)));
}

/**
 * Whether an attribute value is display text rather than a machine token.
 *
 * The rule: **anything with a capital letter or a space is display text; a bare
 * all-lowercase token is not.** `Core`, `Your name` and `Toggle fullscreen` are
 * caught; `close`, `nav-back` and `camera_fore` are not.
 *
 * That is the honest line. Capitalisation is the only signal English markup
 * actually gives — a designer writing a tooltip capitalises it, a developer
 * writing a DOM hook does not. The deliberate blind spot is the other corner:
 * `label="core"`, an all-lowercase single word that IS shown, reads as a token
 * and is missed. Widening past that would flag every hook in the client, and a
 * gate that cries wolf gets a blanket suppression, which is worse than no gate.
 *
 * A value that INTERPOLATES is exempt from the token test: `title="${x} systems"`
 * is composed prose whatever case its literal half is in, and a composed value
 * cannot be a single machine token.
 *
 * @param {string} value raw attribute value, `${…}` included
 * @returns {boolean}
 */
export function isDisplayAttrValue(value) {
  const composed = value.includes('${');
  const bare = stripEntities(stripInterpolations(value)).trim();
  if (!HAS_WORD.test(bare)) return false;
  if (!composed && MACHINE_TOKEN.test(bare)) return false;
  return true;
}

// ── JS lexing ───────────────────────────────────────────────────────────────

/**
 * This is a scanner, not a parser, and the gap is the dangerous part: it lexes
 * only enough JS to tell a template literal from a comment, a string and a
 * regex. Where it is NOT confident it says so — every helper below takes a
 * `problems` sink, and a mis-lex surfaces as an `unscannable` finding.
 *
 * That asymmetry is the whole point. A scan that returns zero findings because
 * it stopped looking reports the same green as one that looked and found
 * nothing, and telling those two apart is the only reason this gate exists.
 * Before the regex handling below, `s.replace(/'/g, '')` on a line made the
 * rest of that line — template literal included — invisible, silently.
 *
 * @typedef {{ index: number, reason: string }} LexProblem
 */

/**
 * Keywords after which a `/` opens a regex literal rather than dividing.
 * `return /x/` is a regex; `count /x/` is two divisions.
 */
const REGEX_AFTER_KEYWORD = new Set([
  'await', 'case', 'delete', 'do', 'else', 'in', 'instanceof', 'new',
  'of', 'return', 'throw', 'typeof', 'void', 'yield',
]);

/** Last character of a *value*. After one of these a `/` divides. */
const VALUE_END = /[A-Za-z0-9_$)\]}]/;

/**
 * Whether the `/` at `at` opens a regex literal rather than dividing.
 *
 * The standard heuristic: a regex may only appear where an expression may
 * start, so look back at the last significant character. An identifier, a
 * number or a closing bracket ends a value, and a value cannot be followed by
 * a regex — except when that "identifier" is really a keyword.
 *
 * Deliberately not exact: `if (a) {} /re/.test(s)` reads `}` as a value end and
 * calls it division. That mis-read is now *loud* rather than silent — if the
 * regex holds a quote, `skipQuoted` hits the line end and files a problem.
 */
function startsRegex(src, at) {
  for (let i = at - 1; i >= 0; i -= 1) {
    const c = src[i];
    if (c === ' ' || c === '\t' || c === '\r' || c === '\n') continue;
    if (!VALUE_END.test(c)) return true;
    const word = src.slice(0, i + 1).match(/[A-Za-z_$][\w$]*$/);
    return word !== null && REGEX_AFTER_KEYWORD.has(word[0]);
  }
  return true; // start of source
}

/** Index just past the closing `/` of the regex literal starting at `at`. */
function skipRegex(src, at, problems) {
  let i = at + 1;
  let inClass = false;
  while (i < src.length) {
    const c = src[i];
    if (c === '\\') { i += 2; continue; }
    if (c === '\n') {
      problems?.push({ index: at, reason: 'unterminated regex literal' });
      return i;
    }
    if (inClass) { if (c === ']') inClass = false; }
    else if (c === '[') inClass = true;
    else if (c === '/') return i + 1; // flags are identifier chars; harmless
    i += 1;
  }
  problems?.push({ index: at, reason: 'unterminated regex literal' });
  return i;
}

/**
 * Index just past the closing quote of the string starting at `at`.
 *
 * @param {string} src
 * @param {number} at
 * @param {LexProblem[]} [problems]
 */
function skipQuoted(src, at, problems) {
  const quote = src[at];
  let i = at + 1;
  while (i < src.length) {
    if (src[i] === '\\') { i += 2; continue; }
    if (src[i] === quote) return i + 1;
    // A JS string cannot span a line. Reaching one means this scanner mistook
    // an apostrophe or a regex slash for a delimiter, so resync at the line end
    // rather than swallow the rest of the file — and SAY so, because everything
    // between here and there went unscanned.
    if (src[i] === '\n') {
      problems?.push({ index: at, reason: `unterminated ${quote} string` });
      return i;
    }
    i += 1;
  }
  problems?.push({ index: at, reason: `unterminated ${quote} string` });
  return i;
}

/**
 * Index just past the `}` closing the `${` at `at`.
 *
 * @param {string} src
 * @param {number} at
 * @param {LexProblem[]} [problems]
 */
function skipInterpolation(src, at, problems) {
  let i = at + 2;
  let depth = 1;
  while (i < src.length && depth > 0) {
    const c = src[i];
    if (c === '/' && src[i + 1] === '/') {
      const nl = src.indexOf('\n', i);
      i = nl === -1 ? src.length : nl + 1;
      continue;
    }
    if (c === '/' && src[i + 1] === '*') {
      const end = src.indexOf('*/', i + 2);
      i = end === -1 ? src.length : end + 2;
      continue;
    }
    if (c === '/' && startsRegex(src, i)) { i = skipRegex(src, i, problems); continue; }
    if (c === "'" || c === '"') { i = skipQuoted(src, i, problems); continue; }
    if (c === '`') { i = readTemplate(src, i, problems).end; continue; }
    if (c === '{') depth += 1;
    else if (c === '}') depth -= 1;
    i += 1;
  }
  if (depth > 0) problems?.push({ index: at, reason: 'unterminated ${…} interpolation' });
  return i;
}

/**
 * Read the template literal whose backtick is at `at`.
 *
 * The body keeps each `${…}` VERBATIM so every offset inside it still lines up
 * with the original source — that is what lets a finding be reported at its
 * real line rather than at the top of the template. `blankInterpolations`
 * erases the JS inside them later, at the same widths.
 *
 * @param {string} src
 * @param {number} at
 * @param {LexProblem[]} [problems]
 * @returns {{ text: string, end: number, interpolations: {start: number, end: number}[] }}
 */
function readTemplate(src, at, problems) {
  let i = at + 1;
  let text = '';
  const interpolations = [];
  while (i < src.length && src[i] !== '`') {
    if (src[i] === '\\') { text += src.slice(i, i + 2); i += 2; continue; }
    if (src[i] === '$' && src[i + 1] === '{') {
      const start = i;
      i = skipInterpolation(src, i, problems);
      text += src.slice(start, i);
      interpolations.push({ start: start + 2, end: i - 1 });
      continue;
    }
    text += src[i];
    i += 1;
  }
  if (i >= src.length) problems?.push({ index: at, reason: 'unterminated template literal' });
  return { text, end: i + 1, interpolations };
}

/**
 * Every template literal in a JS source, outermost first, then any nested
 * inside an interpolation (`${rows.map((r) => `<li>…</li>`).join('')}`) — those
 * would otherwise vanish, since `blankInterpolations` erases the span they sit
 * in.
 *
 * Comments, ordinary quoted strings and regex literals are skipped so a
 * backtick or a quote inside any of them is not mistaken for a delimiter.
 *
 * @param {string} src
 * @returns {{ templates: { text: string, start: number }[], problems: LexProblem[] }}
 *   `start` indexes the first body character; a non-empty `problems` means part
 *   of this source was NOT scanned.
 */
export function scanTemplates(src) {
  const templates = [];
  /** @type {LexProblem[]} */
  const problems = [];
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    // Comments are tested before regexes: `//` is never a valid (empty) regex,
    // and testing the other way round would lex every line comment sitting in
    // an expression position as an unterminated regex literal.
    if (c === '/' && src[i + 1] === '/') {
      const nl = src.indexOf('\n', i);
      i = nl === -1 ? src.length : nl + 1;
      continue;
    }
    if (c === '/' && src[i + 1] === '*') {
      const end = src.indexOf('*/', i + 2);
      i = end === -1 ? src.length : end + 2;
      continue;
    }
    if (c === '/' && startsRegex(src, i)) { i = skipRegex(src, i, problems); continue; }
    if (c === "'" || c === '"') { i = skipQuoted(src, i, problems); continue; }
    if (c === '`') {
      const lit = readTemplate(src, i, problems);
      templates.push({ text: lit.text, start: i + 1 });
      for (const span of lit.interpolations) {
        const nested = scanTemplates(src.slice(span.start, span.end));
        for (const t of nested.templates) {
          templates.push({ text: t.text, start: span.start + t.start });
        }
        for (const p of nested.problems) {
          problems.push({ index: span.start + p.index, reason: p.reason });
        }
      }
      i = lit.end;
      continue;
    }
    i += 1;
  }
  return { templates, problems };
}

/**
 * Blank the JS inside every `${…}`, keeping the delimiters, the width and the
 * newlines.
 *
 * `readTemplate` keeps interpolations verbatim so offsets survive, which leaves
 * live JS in what `markupFindings` then reads as markup: a nested template's
 * `<li>` parses as a real tag, and the `).join('')}` after its closing backtick
 * parses as a TEXT NODE — a hardcoded-English warning pointing at no English at
 * all, and CI red on correct code. Blanking removes the JS without moving a
 * single character, so a finding still lands on its real line.
 *
 * The `${` and `}` survive because `isDisplayAttrValue` reads them: a value
 * that interpolates is composed prose, whatever case its literal half is in.
 *
 * @param {string} s
 * @returns {string}
 */
export function blankInterpolations(s) {
  let out = '';
  let i = 0;
  while (i < s.length) {
    if (s[i] === '$' && s[i + 1] === '{') {
      const end = skipInterpolation(s, i);
      const span = s.slice(i, end);
      const closed = span.endsWith('}');
      out += '${';
      for (let k = 2; k < (closed ? span.length - 1 : span.length); k += 1) {
        out += span[k] === '\n' ? '\n' : ' ';
      }
      if (closed) out += '}';
      i = end;
      continue;
    }
    out += s[i];
    i += 1;
  }
  return out;
}

/** Whether a template literal is markup rather than a css block or a message. */
export function isHtmlTemplate(text) {
  return /<[a-zA-Z][\w-]*(?:[\s/>])/.test(text);
}

// ── Markup scan ─────────────────────────────────────────────────────────────

/**
 * Attributes of a start tag, as `name → { value, index }` with `index` relative
 * to the start of `raw`.
 */
function parseAttributes(raw) {
  const attrs = new Map();
  const pattern = /([a-zA-Z_:][\w:.-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))/g;
  for (const m of raw.matchAll(pattern)) {
    const value = m[2] ?? m[3] ?? m[4] ?? '';
    const quoted = m[0].endsWith('"') || m[0].endsWith("'");
    attrs.set(m[1].toLowerCase(), {
      value,
      index: m.index + m[0].length - value.length - (quoted ? 1 : 0),
    });
  }
  return attrs;
}

/** Attribute names an element's `data-i18n-attr` already resolves at runtime. */
function localisedAttrs(attrs) {
  const covered = new Set();
  const decl = attrs.get('data-i18n-attr');
  if (!decl) return covered;
  for (const pair of decl.value.split(',')) {
    const sep = pair.indexOf(':');
    if (sep !== -1) covered.add(pair.slice(0, sep).trim().toLowerCase());
  }
  return covered;
}

/**
 * Un-localised display text in a chunk of markup.
 *
 * A text node is exempt when ANY ancestor carries `data-i18n`, because
 * `applyToDom` sets that element's whole `textContent` — the literal in the
 * markup is the English fallback, not a missed string.
 *
 * That exemption is only true where `applyToDom` actually runs, which is over
 * `document`, once, at boot (client.html, server.html, gui/console-core.js).
 * Nothing calls it on a shadowRoot or re-runs it after markup is built from a
 * template literal, so in JS-built markup `data-i18n` resolves NOTHING while
 * still silencing this scan — hardcoded English, shipped green. Callers pass
 * `i18nApplies: false` for that case and get one `inert-i18n` finding per
 * tagged element instead of silence.
 *
 * `<script>` and `<style>` bodies are skipped: their contents are code, and a
 * css rule is wall-to-wall lowercase words.
 *
 * @param {string} html
 * @param {{ i18nApplies?: boolean }} [options]
 * @returns {{ kind: 'text'|'attr'|'inert-i18n', attr: string|null, text: string, index: number }[]}
 */
export function markupFindings(html, { i18nApplies = true } = {}) {
  const findings = [];
  /** @type {{ name: string, i18n: boolean }[]} */
  const stack = [];
  let i = 0;

  const emitText = (chunk, index) => {
    if (stack.some((e) => e.i18n)) return;
    if (!isMarkupText(chunk)) return;
    findings.push({ kind: 'text', attr: null, text: chunk.trim(), index });
  };

  while (i < html.length) {
    const lt = html.indexOf('<', i);
    if (lt === -1) { emitText(html.slice(i), i); break; }
    emitText(html.slice(i, lt), i);

    if (html.startsWith('<!--', lt)) {
      const end = html.indexOf('-->', lt);
      i = end === -1 ? html.length : end + 3;
      continue;
    }
    if (html.startsWith('<!', lt)) { // doctype
      const end = html.indexOf('>', lt);
      i = end === -1 ? html.length : end + 1;
      continue;
    }

    // Walk to the tag's own `>`, ignoring one inside a quoted attribute value.
    let j = lt + 1;
    let quote = null;
    for (; j < html.length; j += 1) {
      const c = html[j];
      if (quote) { if (c === quote) quote = null; continue; }
      if (c === '"' || c === "'") { quote = c; continue; }
      if (c === '>') break;
    }
    const raw = html.slice(lt + 1, j);
    const named = raw.match(/^(\/?)\s*([a-zA-Z][\w-]*)/);
    if (!named) { i = j + 1; continue; } // a bare `<` in prose

    const name = named[2].toLowerCase();
    if (named[1] === '/') {
      const at = stack.map((e) => e.name).lastIndexOf(name);
      if (at !== -1) stack.length = at;
      i = j + 1;
      continue;
    }

    const attrs = parseAttributes(raw);
    const covered = localisedAttrs(attrs);
    // The exemptions below are runtime promises. Where nothing keeps them, say
    // so once, at the tag — rather than let them buy silence for the subtree.
    if (!i18nApplies) {
      for (const decl of ['data-i18n', 'data-i18n-attr']) {
        const found = attrs.get(decl);
        if (found) {
          findings.push({
            kind: 'inert-i18n', attr: decl, text: found.value, index: lt + 1 + found.index,
          });
        }
      }
    }
    for (const [attr, { value, index }] of attrs) {
      if (!TEXT_BEARING_ATTRS.has(attr) || covered.has(attr)) continue;
      if (!isDisplayAttrValue(value)) continue;
      findings.push({ kind: 'attr', attr, text: value, index: lt + 1 + index });
    }

    if (RAW_TEXT_ELEMENTS.has(name)) {
      const close = html.toLowerCase().indexOf(`</${name}`, j);
      i = close === -1 ? html.length : close;
      continue;
    }
    if (!VOID_ELEMENTS.has(name) && !raw.trimEnd().endsWith('/')) {
      stack.push({ name, i18n: attrs.has('data-i18n') });
    }
    i = j + 1;
  }

  return findings;
}

/**
 * Un-localised display text in a client source file.
 *
 * An `.html` file is markup outright; a `.js` file is markup only inside the
 * template literals a component builds its shadow DOM from. An `.html` file's
 * inline `<script>` is scanned BOTH ways — skipped as markup, then mined for
 * templates — because a console's inline module builds markup exactly the way
 * a component does.
 *
 * An `unscannable` finding means the JS lexer lost its place: a region of this
 * file was NOT scanned, so nothing here is evidence of anything. It is reported
 * exactly like un-localised text, because a gate that cannot read a file must
 * not report the same green as one that read it and found nothing.
 *
 * @param {string} src
 * @param {boolean} isHtml
 * @returns {{ kind: 'text'|'attr'|'inert-i18n'|'unscannable', attr: string|null, text: string, index: number }[]}
 */
export function untranslatedMarkup(src, isHtml) {
  const findings = [];
  /** @type {{ code: string, start: number }[]} */
  const scripts = [];

  if (isHtml) {
    findings.push(...markupFindings(src));
    // Only the <script> bodies go to the JS scanner. Handing it the whole
    // document would have it read every apostrophe in the prose as a string
    // delimiter and every `<div>` as division.
    for (const open of src.matchAll(/<script\b[^>]*>/gi)) {
      const start = open.index + open[0].length;
      const close = src.toLowerCase().indexOf('</script', start);
      scripts.push({ code: src.slice(start, close === -1 ? src.length : close), start });
    }
  } else {
    scripts.push({ code: src, start: 0 });
  }

  // A nested template is lexed twice — once by the enclosing interpolation,
  // once by the recursion that reaches it — so the same mis-lex arrives twice.
  const seen = new Set();

  for (const { code, start } of scripts) {
    const { templates, problems } = scanTemplates(code);
    for (const p of problems) {
      const index = start + p.index;
      const key = `${index}:${p.reason}`;
      if (seen.has(key)) continue;
      seen.add(key);
      findings.push({ kind: 'unscannable', attr: null, text: p.reason, index });
    }
    for (const tpl of templates) {
      if (!isHtmlTemplate(tpl.text)) continue;
      // Blank the interpolations first: they hold JS, and markupFindings would
      // otherwise read a nested template's tags as markup and the code after
      // its closing backtick as a text node. Widths are preserved, so the
      // offsets below still land on the real line.
      for (const f of markupFindings(blankInterpolations(tpl.text), { i18nApplies: false })) {
        findings.push({ ...f, index: start + tpl.start + f.index });
      }
    }
  }

  return findings.sort((a, b) => a.index - b.index);
}

/**
 * 1-based line of `index` in `src`.
 * @param {string} src
 * @param {number} index
 */
export function lineOf(src, index) {
  let line = 1;
  for (let i = 0; i < index && i < src.length; i += 1) if (src[i] === '\n') line += 1;
  return line;
}
