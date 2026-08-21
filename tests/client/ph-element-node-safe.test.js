/**
 * tests/client/ph-element-node-safe.test.js — gui/components/ph-element.js
 * (issue #1231, T4.C0 of the console-seam programme).
 *
 * Deliberately no environment-override pragma at the top of this file: it
 * runs under vitest.config.js's default `environment: 'node'`, where
 * `HTMLElement`, `document`, and `customElements` do not exist — the exact
 * condition `console-core.test.js` already relies on to prove
 * `ph-tutorial-overlay.js`'s Node guard works (it imports `console-core.js`,
 * which imports that component, in plain Node).
 *
 * (Spelling out the actual pragma syntax in this comment would work too
 * well: Vitest's per-file environment scanner matches it anywhere in the
 * source, prose or not, and would flip this whole file to jsdom — which
 * happened once while drafting this file and cost a debugging detour.)
 *
 * `PhElement` is the base every future `ph-*` component will extend, and
 * this issue's whole point is "get the base-class contract right" before
 * anything is built on it — so it gets its own direct proof, rather than
 * relying on some other module to import it transitively. Merely importing
 * this module (never instantiating `PhElement` — that needs a real DOM,
 * which plain Node correctly does not have) must not throw.
 */
import { describe, it, expect } from 'vitest';
import { PhElement, phDefine } from '../../gui/components/ph-element.js';

describe('PhElement — Node-safe import', () => {
  it('confirms this test actually runs without browser globals', () => {
    expect(typeof HTMLElement).toBe('undefined');
    expect(typeof document).toBe('undefined');
    expect(typeof customElements).toBe('undefined');
  });

  it('imports cleanly and exports a class plus the phDefine helper', () => {
    expect(typeof PhElement).toBe('function');
    expect(typeof phDefine).toBe('function');
  });

  it('lets a subclass be declared (class syntax only — no instantiation) without throwing', () => {
    expect(() => {
      class PhNodeSafeProbe extends PhElement {
        template() { return '<div></div>'; }
      }
      return PhNodeSafeProbe;
    }).not.toThrow();
  });

  it('phDefine no-ops instead of throwing when customElements does not exist', () => {
    class PhNodeSafeProbe extends PhElement {}
    expect(() => phDefine('ph-node-safe-probe', PhNodeSafeProbe)).not.toThrow();
  });
});
