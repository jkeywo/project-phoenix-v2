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
