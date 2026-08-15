/**
 * tests/client/build-flags.test.js — the demo-build detection ladder and the
 * restrictions that hang off it (issues #939/#940, PRD #855).
 *
 * `gui/build-flags.js` is DOM/window-free at import time on purpose, so all of
 * this runs in Node with fabricated `doc`/`win` stand-ins and no jsdom.
 */

import { beforeEach, describe, expect, it } from 'vitest';
import {
  buildFlagOverride,
  demoFromMeta,
  isDemoBuild,
  offersModPackUpload,
  setBuildFlags,
} from '../../gui/build-flags.js';

/** A `document` stand-in carrying (or not carrying) the demo meta tag. */
function docWithMeta(content) {
  return {
    querySelector(selector) {
      if (selector !== 'meta[name="phoenix-build-demo"]') return null;
      if (content === undefined) return null;
      return { getAttribute: (name) => (name === 'content' ? content : null) };
    },
  };
}

const NO_DOC = { querySelector: () => null };

beforeEach(() => {
  setBuildFlags({ demo: null });
});

describe('the demo-build detection ladder', () => {
  it('reads the stamped meta tag, treating exactly "true" as the demo', () => {
    expect(demoFromMeta(docWithMeta('true'))).toBe(true);
    expect(demoFromMeta(docWithMeta('false'))).toBe(false);
    expect(demoFromMeta(docWithMeta('TRUE'))).toBe(false);
    expect(demoFromMeta(docWithMeta(undefined))).toBe(null);
  });

  it('treats an unknown build as dev, so development keeps its debug tooling', () => {
    expect(isDemoBuild({ doc: NO_DOC, win: {} })).toBe(false);
  });

  it('accepts either source saying demo, because each is silent at a different moment', () => {
    // The tag alone — the phone page, which has no WASM to ask.
    expect(isDemoBuild({ doc: docWithMeta('true'), win: {} })).toBe(true);
    // The compiled getter alone — a locally-built demo, which carries no tag.
    expect(
      isDemoBuild({ doc: NO_DOC, win: { wasm_is_demo_build: () => true } }),
    ).toBe(true);
  });

  it('does not let a throwing getter promote a demo build back to dev', () => {
    const win = {
      wasm_is_demo_build() {
        throw new Error('WASM not up yet');
      },
    };
    expect(isDemoBuild({ doc: docWithMeta('true'), win })).toBe(true);
  });

  it('lets an explicit override win outright, in both directions', () => {
    setBuildFlags({ demo: true });
    expect(buildFlagOverride()).toBe(true);
    expect(isDemoBuild({ doc: NO_DOC, win: {} })).toBe(true);
    setBuildFlags({ demo: false });
    expect(
      isDemoBuild({ doc: docWithMeta('true'), win: { wasm_is_demo_build: () => true } }),
    ).toBe(false);
  });
});

describe('the mod-pack upload restriction (PRD #855)', () => {
  it('is offered on a dev build', () => {
    expect(offersModPackUpload({ doc: NO_DOC, win: {} })).toBe(true);
  });

  it('is withheld from a demo build, whichever source identifies it', () => {
    expect(offersModPackUpload({ doc: docWithMeta('true'), win: {} })).toBe(false);
    expect(
      offersModPackUpload({ doc: NO_DOC, win: { wasm_is_demo_build: () => true } }),
    ).toBe(false);
  });

  it('tracks the demo answer exactly — it is the same decision, not a second one', () => {
    for (const env of [
      { doc: NO_DOC, win: {} },
      { doc: docWithMeta('true'), win: {} },
      { doc: docWithMeta('false'), win: { wasm_is_demo_build: () => true } },
    ]) {
      expect(offersModPackUpload(env)).toBe(!isDemoBuild(env));
    }
  });
});
