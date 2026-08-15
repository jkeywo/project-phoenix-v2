/**
 * tests/client/console-tokens.test.js — no reference to a token nothing defines.
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
 * something in scope defines. The companion rule — that a stylesheet may not
 * hardcode a value instead of naming a token — lives in design-tokens.test.js.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import {
  GUI, TOKENS_CSS, REPO_ROOT, readStripped, definedProps,
  referencedWithoutFallback, setAtRuntime, rel,
} from './css-scan.js';

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

const rootBlock = fs.readFileSync(TOKENS_CSS, 'utf8').match(/:root\s*\{([\s\S]*)\}/);
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
    it(`${rel(file)} resolves every var(--x) it writes without a fallback`, () => {
      const source = readStripped(file);
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

describe('the lobby speaks the shared vocabulary', () => {
  it('no longer defines a parallel token set of its own', () => {
    // client.html used to carry its own `:root`, with an `--edge` that was a
    // different colour from the consoles' and a `--dim` below the contrast
    // floor. Both retired into gui/tokens.css.
    const lobby = readStripped(path.join(REPO_ROOT, 'client.html'));
    const roots = lobby.match(/:root\s*\{[\s\S]*?\}/g) || [];
    const defined = new Set();
    for (const block of roots) for (const name of definedProps(block)) defined.add(name);
    // The bezel keeps its own handful: they are that one animation's
    // parameters, not vocabulary anything else speaks.
    const allowed = new Set([
      '--bezel-frame', '--bezel-glow-size', '--bezel-pulse-intensity',
      '--bezel-red', '--bezel-inset',
    ]);
    expect([...defined].filter((n) => !allowed.has(n)).sort()).toEqual([]);
  });
});
