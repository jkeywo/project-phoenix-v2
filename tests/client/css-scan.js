/**
 * tests/client/css-scan.js — reading the client's stylesheets as text.
 *
 * Not a test file (no `.test.js`, so vitest does not collect it). It is the
 * shared scanner behind console-tokens.test.js and design-tokens.test.js,
 * which both have to answer the same awkward question: what, in a file that
 * mixes JavaScript, HTML and CSS, is actually a style declaration?
 *
 * The answer is fiddly enough to be worth writing once. Two traps in
 * particular:
 *
 *   - A comment is prose. `// issue #827` is indistinguishable from a
 *     three-digit hex colour, and a design note that names the value it
 *     replaced is worth keeping. Comments are blanked before anything is
 *     scanned, which is why every helper here takes already-stripped source.
 *
 *   - A component's CSS lives inside a JS template literal, so there is no
 *     lexer that will hand back "the stylesheet". Scanning the whole file is
 *     the honest approximation, and it is why the colour scanner looks for
 *     colour SYNTAX rather than trying to find rule bodies.
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
export const GUI = path.join(REPO_ROOT, 'gui');
export const TOKENS_CSS = path.join(GUI, 'tokens.css');

/**
 * Blank out comments, preserving length so offsets and line counts hold.
 *
 * HTML comments go FIRST, and the order is load-bearing. These files nest a
 * `<style>` inside HTML, and a `/* … *\/` in that CSS can straddle an HTML
 * comment's opener — blanking the `<!--` while leaving its `-->` behind, after
 * which the HTML pass finds no opener and the whole comment survives as
 * scannable text. That is not hypothetical: it is how `(issue #463)` in
 * client.html came back as a three-digit colour.
 */
export function stripComments(source) {
  const blank = (m) => m.replace(/[^\n]/g, ' ');
  return source
    .replace(/<!--[\s\S]*?-->/g, blank)
    .replace(/\/\*[\s\S]*?\*\//g, blank)
    .replace(/^[ \t]*\/\/.*$/gm, blank)
    .replace(/[ \t]\/\/[^\n'"`]*$/gm, blank);
}

/** Read a file with its comments blanked. */
export function readStripped(file) {
  return stripComments(fs.readFileSync(file, 'utf8'));
}

/** Custom-property DEFINITIONS: `--name:` (never a `var(--name, …)`). */
export function definedProps(source) {
  const names = new Set();
  const re = /(^|[^-\w])(--[a-z0-9-]+)\s*:/gi;
  let m;
  while ((m = re.exec(source)) !== null) names.add(m[2]);
  return names;
}

/**
 * Custom-property REFERENCES with no fallback: `var(--name)`, not
 * `var(--name, something)`. A fallback is the author saying the property may
 * legitimately be absent.
 */
export function referencedWithoutFallback(source) {
  const names = [];
  const re = /var\(\s*(--[a-z0-9-]+)\s*\)/gi;
  let m;
  while ((m = re.exec(source)) !== null) names.push(m[1]);
  return names;
}

/** Properties assigned from JS at runtime via `setProperty('--x', …)`. */
export function setAtRuntime(source) {
  const names = new Set();
  const re = /setProperty\(\s*['"](--[a-z0-9-]+)['"]/gi;
  let m;
  while ((m = re.exec(source)) !== null) names.add(m[1]);
  return names;
}

/**
 * Hardcoded colour literals: `#rgb`, `#rrggbb`, and numeric `rgb()`/`rgba()`.
 *
 * An `rgba(var(--rgb-cyan), 0.35)` is NOT a literal — alpha is genuinely
 * per-use, and the channel triplet it names is a token.
 */
export function colourLiterals(source) {
  const out = [];
  // `(?<!&)` keeps an HTML numeric entity out of it: `&#8592;` is a left
  // arrow, not a four-digit colour.
  const hex = /(?<!&)#([0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})\b/g;
  let m;
  while ((m = hex.exec(source)) !== null) out.push(m[0]);
  const rgb = /rgba?\(\s*\d+\s*,\s*\d+\s*,\s*\d+/g;
  while ((m = rgb.exec(source)) !== null) out.push(m[0] + ' …)');
  return out;
}

/**
 * Hardcoded FONT SIZES: `font-size: 12px` / `0.6rem` and the shorthand
 * `font: 600 11px …`.
 *
 * Only type sizes, deliberately. The floor this vocabulary exists to hold is
 * a LEGIBILITY floor — the smallest a string may render — and the type ramp is
 * what carries it. A padding or a gap has no floor to cross, so demanding a
 * token for every length in the codebase would be noise that teaches a reader
 * to ignore the rule.
 */
export function fontSizeLiterals(source) {
  const out = [];
  const decl = /font-size\s*:\s*([^;}'"`]+)/gi;
  let m;
  while ((m = decl.exec(source)) !== null) {
    const value = m[1].trim();
    if (!/var\(|inherit|unset|initial|revert/.test(value)) out.push(`font-size: ${value}`);
  }
  const short = /font\s*:\s*(?:[a-z0-9-]+\s+)*?(\d*\.?\d+(?:px|rem|em|pt))\b/gi;
  while ((m = short.exec(source)) !== null) out.push(`font: … ${m[1]}`);
  return out;
}

/**
 * The style RULES in a file, as `{selector, body}`.
 *
 * The same awkward question as everything else here — these files mix JS, HTML
 * and CSS, and there is no lexer that will hand back "the stylesheet". Brace
 * matching alone finds every JS block as well, so a candidate only counts as a
 * rule when its body PARSES as declarations: `prop: value` pairs, semicolon
 * separated, nothing else. A function body fails that on its first statement,
 * which is exactly the discrimination needed and costs no configuration.
 *
 * Nested at-rules (`@media`, `@supports`) are walked into, so a rule inside one
 * is returned with `at` naming the query it sits under. That matters for the
 * reduced-motion check, which is entirely a question of what is inside which
 * `@media`.
 *
 * @param {string} source  already comment-stripped
 * @returns {{selector: string, body: string, at: string|null}[]}
 */
export function cssRules(source) {
  // A component's stylesheet lives inside a template literal inside a class
  // body, so it is never at brace depth 0 — the scan has to work at any depth.
  // `<style>` / `</style>` are boundaries in the HTML files and are turned into
  // one the scan already understands rather than being special-cased below.
  const text = source
    .replace(/<\/?(?:style|script)[^>]*>/gi, (m) => ';'.padEnd(m.length, ' '))
    // Several components build their stylesheet as an ARRAY of one-line
    // strings joined with newlines, which splits a single rule across several
    // JS string literals. Concatenating adjacent literals puts those rules back
    // together; a joined pair that was never CSS still fails the declaration
    // test below, so this costs nothing where it does not apply.
    .replace(/(['"`])\s*,\s*(['"`])/g, ' ');

  const out = [];
  const stack = [];
  let boundary = 0;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (ch === '{') {
      stack.push({ selector: selectorBefore(text, boundary, i), start: i, leaf: true });
      boundary = i + 1;
    } else if (ch === '}') {
      const block = stack.pop();
      boundary = i + 1;
      if (stack.length > 0) stack[stack.length - 1].leaf = false;
      if (!block || !block.leaf) continue;
      const body = text.slice(block.start + 1, i);
      if (!looksLikeDeclarations(body)) continue;
      // An at-rule can only be an ancestor, never this block itself, because a
      // block holding other blocks is not a leaf.
      const at = stack.map((s) => s.selector).reverse().find((s) => s.startsWith('@')) || null;
      out.push({ selector: block.selector, body, at });
    } else if (ch === ';') {
      boundary = i + 1;
    }
  }
  return out;
}

/**
 * The selector immediately before a `{`, given the last structural boundary.
 *
 * Only the LAST line of that span, because everything before it is either
 * blank or the JavaScript this CSS is embedded in — except for a multi-line
 * selector LIST, whose earlier lines are recognisable by ending in a comma.
 *
 * The quote handling earns its keep on the components that build their
 * stylesheet as an array of strings: a line reading `'.corner-label {'` leaves
 * an unbalanced opening quote in the span, and cutting at it yields the
 * selector. A line with BALANCED quotes is a genuine attribute selector —
 * `:host([data-state="stalled"]) .fill` — and must be left alone.
 */
function selectorBefore(text, boundary, braceIndex) {
  const span = text.slice(boundary, braceIndex);
  const lines = span.split('\n');
  const kept = [lines[lines.length - 1]];
  for (let i = lines.length - 2; i >= 0; i -= 1) {
    const previous = lines[i].trim();
    // A multi-line selector LIST is the only reason to keep reading upwards,
    // and its earlier lines end in a comma. Three things disqualify a comma:
    // an odd number of quotes (a JS string fragment, not CSS), a bare
    // identifier (an array element like `SCOPE_CHROME_CSS,`), and no comma.
    if (!previous.endsWith(',')) break;
    if ((previous.match(/['"`]/g) || []).length % 2 !== 0) break;
    if (/^[A-Za-z_$][A-Za-z0-9_$]*\s*,$/.test(previous)) break;
    kept.unshift(lines[i]);
  }
  return kept
    .map((line) => {
      const quotes = (line.match(/['"`]/g) || []).length;
      if (quotes % 2 === 0) return line;
      return line.slice(line.lastIndexOf(line.match(/['"`]/g).at(-1)) + 1);
    })
    .join(' ')
    .replace(/\s+/g, ' ')
    .trim();
}

/** Does this block body read as CSS declarations rather than as code? */
function looksLikeDeclarations(body) {
  const text = body.trim();
  if (text === '') return false;
  if (/[{}]/.test(text)) return false;
  const parts = text.split(';').map((p) => p.trim()).filter(Boolean);
  if (parts.length === 0) return false;
  return parts.every((part) => /^[-a-z][-a-z0-9]*\s*:\s*\S/i.test(part));
}

/** The console documents: every per-hull HTML page under gui/. */
export function consoleDocuments() {
  const out = [];
  for (const hull of fs.readdirSync(GUI, { withFileTypes: true })) {
    if (!hull.isDirectory() || hull.name === 'borders' || hull.name === 'components') continue;
    const dir = path.join(GUI, hull.name);
    for (const file of fs.readdirSync(dir)) {
      if (file.endsWith('.html')) out.push(path.join(dir, file));
    }
  }
  return out.sort();
}

/** The shadow-DOM components: gui/components/ph-*.js. */
export function componentFiles() {
  return fs.readdirSync(path.join(GUI, 'components'))
    .filter((f) => f.endsWith('.js'))
    .map((f) => path.join(GUI, 'components', f))
    .sort();
}

/** A repo-relative, forward-slashed path for a readable test name. */
export function rel(file) {
  return path.relative(REPO_ROOT, file).replace(/\\/g, '/');
}
