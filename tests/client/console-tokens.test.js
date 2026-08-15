/**
 * tests/client/console-tokens.test.js — the console token vocabulary.
 *
 * A console page is an IFRAME: a separate document that inherits nothing from
 * client.html. Its shared vocabulary is the `:root` block in gui/tokens.css,
 * which gui/console.css imports and which custom properties DO carry across
 * the shadow-DOM boundary into every `ph-*` component.
 *
 * That makes an undefined custom property invisible until someone looks at the
 * right console: `border: 1px solid var(--edge)` with no `--edge` anywhere is
 * not a parse error — the reference is invalid at computed-value time, so the
 * whole shorthand falls back to its initial value and the border silently is
 * not drawn. Exactly that shipped (PRD #1023's defect list): `--edge` was
 * defined only in client.html's lobby `:root`, and every console document's
 * panel columns rendered borderless.
 *
 * So: every `var(--x)` written without a fallback must resolve to a property
 * something in scope defines.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const GUI = path.join(REPO_ROOT, 'gui');
const TOKENS_CSS = path.join(GUI, 'tokens.css');

/** Every file under gui/ that carries CSS: console documents and components. */
function styledFiles() {
  const out = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name === 'borders') continue; // image assets only
        walk(full);
      } else if (/\.(html|js|css)$/.test(entry.name)) {
        out.push(full);
      }
    }
  };
  walk(GUI);
  return out;
}

/** Custom-property DEFINITIONS in a source: `--name:` outside a `var(` call. */
function definedProps(source) {
  const names = new Set();
  const re = /(^|[^-\w])(--[a-z0-9-]+)\s*:/gi;
  let m;
  while ((m = re.exec(source)) !== null) {
    // `var(--x, ...)` never matches (the comma, not a colon, follows), but a
    // `style.setProperty('--charge', v)` call does not match either — those are
    // picked up by SET_AT_RUNTIME below.
    names.add(m[2]);
  }
  return names;
}

/**
 * Custom-property REFERENCES with no fallback: `var(--name)` but not
 * `var(--name, something)`. A fallback is the author saying "this may be
 * absent", which is a deliberate, safe choice and not what this test polices.
 */
function referencedWithoutFallback(source) {
  const names = [];
  const re = /var\(\s*(--[a-z0-9-]+)\s*\)/gi;
  let m;
  while ((m = re.exec(source)) !== null) names.push(m[1]);
  return names;
}

/** Properties assigned from JS at runtime via setProperty('--x', …). */
function setAtRuntime(source) {
  const names = new Set();
  const re = /setProperty\(\s*['"](--[a-z0-9-]+)['"]/gi;
  let m;
  while ((m = re.exec(source)) !== null) names.add(m[1]);
  return names;
}

const tokensSource = fs.readFileSync(TOKENS_CSS, 'utf8');
const rootBlock = tokensSource.match(/:root\s*\{([\s\S]*)\}/);
const SHARED_TOKENS = definedProps(rootBlock ? rootBlock[1] : '');

describe('gui/tokens.css :root token vocabulary', () => {
  it('is the shared vocabulary every console document and component inherits', () => {
    expect(SHARED_TOKENS.size).toBeGreaterThan(10);
  });

  it('defines --edge, the panel border every console document draws', () => {
    // The defect: ~40 `border: 1px solid var(--edge)` declarations across the
    // console documents against a token only client.html defined. A console
    // iframe inherits nothing from client.html, so every one of those borders
    // silently resolved to `none`.
    expect(SHARED_TOKENS.has('--edge')).toBe(true);
  });
});

describe('no console document or component references an undefined token', () => {
  const files = styledFiles();

  it('finds the console documents and components to check', () => {
    expect(files.length).toBeGreaterThan(30);
  });

  for (const file of files) {
    const rel = path.relative(REPO_ROOT, file).replace(/\\/g, '/');
    it(`${rel} resolves every var(--x) it writes without a fallback`, () => {
      const source = fs.readFileSync(file, 'utf8');
      // In scope: the shared :root tokens, anything this file defines itself
      // (component-local geometry like --btn-h, --cham, --hero-bar), and
      // anything it assigns from JS at runtime (--charge).
      const inScope = new Set([
        ...SHARED_TOKENS,
        ...definedProps(source),
        ...setAtRuntime(source),
      ]);
      const missing = [...new Set(referencedWithoutFallback(source))]
        .filter((name) => !inScope.has(name));
      expect(missing).toEqual([]);
    });
  }
});
