/**
 * tests/client/design-tokens.test.js — module 1's regression suite (PRD #1023).
 *
 * The PRD's contract, in its own words: "a test walks every component and
 * console stylesheet asserting adoption of the shared tokens and the absence
 * of hardcoded colour/size literals outside the token file".
 *
 * Three rules, then, and each of them is here because the codebase had already
 * broken it:
 *
 *   1. ADOPTION. Every ph-* component adopts the shared control family. Five
 *      of thirty-six did; the other thirty-one hand-rolled their chrome, which
 *      is how the fleet acquired buttons that agreed on nothing.
 *
 *   2. NO COLOUR LITERALS. 389 of them, across the components, the console
 *      documents and the lobby — including two greys that differed by one
 *      channel and an `--edge` that was navy in a console and near-black in
 *      the lobby.
 *
 *   3. NO TYPE-SIZE LITERALS. The console root is `clamp(11px, 3vw, 15px)`, so
 *      the `0.6rem` labels the consoles are full of rendered at 6.6px on a
 *      narrow phone. Sizes come from the type ramp, whose rungs are `max()`
 *      against an absolute floor.
 *
 * ── What is deliberately NOT policed ───────────────────────────────────────
 *
 * Comments. `// issue #827` is indistinguishable from a three-digit hex, and a
 * note recording the value a token replaced is worth keeping. See css-scan.js.
 *
 * Lengths that are not type sizes. Padding, gaps and border widths have no
 * legibility floor to cross, and demanding a token for every length in the
 * codebase would be noise — the kind of rule a reader learns to suppress,
 * which then hides the rule that mattered.
 *
 * gui/tokens.css itself, which is where the values are supposed to live, and
 * the exemptions named in EXEMPT below, each with its reason.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import {
  REPO_ROOT, GUI, TOKENS_CSS, readStripped, definedProps, colourLiterals,
  fontSizeLiterals, consoleDocuments, componentFiles, rel,
} from './css-scan.js';

const TOKENS = fs.readFileSync(TOKENS_CSS, 'utf8');

/**
 * Files that may still hold a raw value, and why.
 *
 * A short list with a reason each, rather than a blanket rule, so that adding
 * to it is a visible decision.
 */
const EXEMPT = new Set([
  // The vocabulary itself. This is where the values live.
  'gui/tokens.css',
]);

// ── 1. The vocabulary ───────────────────────────────────────────────────────

describe('the token vocabulary', () => {
  const root = TOKENS.match(/:root\s*\{([\s\S]*)\}/);
  const names = definedProps(root ? root[1] : '');

  it('is defined in exactly one place', () => {
    // Every other stylesheet in the client gets it by import, link or
    // inheritance. A second `:root` block anywhere is the drift starting again.
    const others = [
      path.join(GUI, 'console.css'),
      path.join(REPO_ROOT, 'client.html'),
    ];
    for (const file of others) {
      const blocks = readStripped(file).match(/:root\s*\{[\s\S]*?\}/g) || [];
      const defined = new Set();
      for (const b of blocks) for (const n of definedProps(b)) defined.add(n);
      // client.html keeps the red-alert bezel's own animation parameters.
      const allowed = /^--bezel-/;
      expect([...defined].filter((n) => !allowed.test(n)).sort()).toEqual([]);
    }
  });

  it('names the ramps the PRD asked for: edges, ink, accents, space, type', () => {
    for (const required of [
      '--edge', '--edge-control', '--edge-faint',
      '--ink', '--ink-dim', '--ink-faint',
      '--tactical', '--fire', '--loaded', '--reloading', '--cyan',
      '--space-1', '--space-4', '--space-7',
      '--text-min', '--text-xs', '--text-sm', '--text-md', '--text-lg',
      '--control-h-sm', '--control-h-md', '--control-h-lg', '--control-hit-min',
    ]) {
      expect(names.has(required)).toBe(true);
    }
  });

  it('gives the type ramp a legibility floor that a viewport cannot undercut', () => {
    // A rung written as a bare `rem` shrinks with the console root, which is
    // `clamp(11px, 3vw, 15px)` — that is how 0.6rem labels came to render at
    // 6.6px. Every rung is a max() against an absolute minimum instead.
    const ramp = TOKENS.match(/--text-(?:xs|sm|md|lg|xl|2xl|display):\s*([^;]+);/g) || [];
    expect(ramp.length).toBeGreaterThanOrEqual(5);
    for (const rung of ramp) expect(rung).toMatch(/max\(/);
  });
});

// ── 2. Contrast ─────────────────────────────────────────────────────────────

describe('contrast-bearing tokens meet WCAG AA', () => {
  const hex = (name) => {
    const m = TOKENS.match(new RegExp(`${name}:\\s*(#[0-9a-fA-F]{6})`));
    return m ? m[1] : null;
  };
  const channel = (c) => {
    const v = c / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  const luminance = (h) => {
    const [r, g, b] = [1, 3, 5].map((i) => channel(parseInt(h.slice(i, i + 2), 16)));
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const ratio = (a, b) => {
    const [la, lb] = [luminance(a), luminance(b)];
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
  };

  const BASE = '#0a1028'; // --surface-base, the reference background

  it('measures against the surface the consoles actually paint on', () => {
    expect(hex('--surface-base')).toBe(BASE);
  });

  // 4.5:1 — these are all used as TEXT, at sizes down to the type floor.
  for (const name of ['--ink', '--ink-dim', '--ink-faint', '--tactical', '--fire',
    '--fire-bright', '--loaded', '--reloading', '--cyan', '--gold', '--signal',
    '--sky', '--science']) {
    it(`${name} clears 4.5:1 as text on --surface-base`, () => {
      const value = hex(name);
      expect(value).not.toBeNull();
      expect(ratio(value, BASE)).toBeGreaterThanOrEqual(4.5);
    });
  }

  it('--edge-control clears the 3:1 floor WCAG 1.4.11 asks of a control boundary', () => {
    expect(ratio(hex('--edge-control'), BASE)).toBeGreaterThanOrEqual(3);
  });

  it('documents why the content edges are exempt rather than leaving it silent', () => {
    // `--edge` divides content — a panel column's border. The column is
    // legible without it, so it carries no information a player must perceive,
    // and it stays at the authored navy. That reasoning has to be written
    // down, or the next person reads a failing floor as an oversight.
    expect(TOKENS).toMatch(/1\.4\.11/);
    expect(TOKENS).toMatch(/exempt/i);
  });
});

// ── 2b. The high-contrast palette (issue #1171) ─────────────────────────────

describe('data-contrast="more" swaps in a genuine high-contrast palette', () => {
  // The accessibility profile stamps `data-contrast="more"` on every root
  // (shell + each console iframe); this block is where that attribute becomes
  // a visible palette. It redefines the base rungs under
  // `:root[data-contrast="more"]`, whose specificity beats the bare `:root`.
  const block = (() => {
    const m = TOKENS.match(/:root\[data-contrast="more"\]\s*\{([\s\S]*?)\}/);
    return m ? m[1] : null;
  })();

  const hexIn = (src, name) => {
    const m = src && src.match(new RegExp(`${name}:\\s*(#[0-9a-fA-F]{6})`));
    return m ? m[1] : null;
  };
  const channel = (c) => {
    const v = c / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  const luminance = (h) => {
    const [r, g, b] = [1, 3, 5].map((i) => channel(parseInt(h.slice(i, i + 2), 16)));
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const ratio = (a, b) => {
    const [la, lb] = [luminance(a), luminance(b)];
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
  };

  it('exists as a block that overrides the base :root', () => {
    expect(block).not.toBeNull();
  });

  const HC_BASE = hexIn(block, '--surface-base'); // the high-contrast reference bg

  it('drops the reference background to pure black', () => {
    expect(HC_BASE).toBe('#000000');
  });

  // Every text/signal rung clears WCAG AAA (7:1) on the high-contrast surface —
  // a strictly higher bar than the standard palette's 4.5:1 floor.
  for (const name of ['--ink', '--ink-dim', '--ink-faint', '--tactical', '--fire',
    '--loaded', '--reloading', '--cyan', '--gold', '--signal', '--sky', '--science',
    '--violet']) {
    it(`${name} clears 7:1 (AAA) as text on the high-contrast background`, () => {
      const value = hexIn(block, name);
      expect(value).not.toBeNull();
      expect(ratio(value, HC_BASE)).toBeGreaterThanOrEqual(7);
    });
  }

  // The content dividers the standard palette exempts (below the 3:1 UI floor)
  // are raised here to be plainly visible — region separation moves onto them
  // now that the surfaces are flat black.
  for (const name of ['--edge-faint', '--edge', '--edge-control']) {
    it(`${name} is a clearly visible border (>= 3:1) under high contrast`, () => {
      const value = hexIn(block, name);
      expect(value).not.toBeNull();
      expect(ratio(value, HC_BASE)).toBeGreaterThanOrEqual(3);
    });
  }

  it('raises contrast versus the standard palette — a visible change, not a copy', () => {
    // The standard palette's dimmest divider and faint ink sit far lower; the
    // high-contrast variant must measurably out-contrast them on its own bg.
    const stdBase = TOKENS.match(/--surface-base:\s*(#[0-9a-fA-F]{6})/)[1];
    const stdFaintEdge = TOKENS.match(/--edge-faint:\s*(#[0-9a-fA-F]{6})/)[1];
    const stdFaintInk = TOKENS.match(/--ink-faint:\s*(#[0-9a-fA-F]{6})/)[1];
    expect(ratio(hexIn(block, '--edge-faint'), HC_BASE))
      .toBeGreaterThan(ratio(stdFaintEdge, stdBase));
    expect(ratio(hexIn(block, '--ink-faint'), HC_BASE))
      .toBeGreaterThan(ratio(stdFaintInk, stdBase));
  });

  it('keeps the keyboard focus ring visible by swapping in its high-contrast half', () => {
    // #1170 defined the pair; here `--focus-ring` resolves to the contrast half
    // so the ring stays unmistakable on the raised palette.
    expect(block).toMatch(/--focus-ring:\s*var\(--focus-ring-contrast\)/);
    // And that half is itself maximally legible on the high-contrast bg.
    const contrastRing = TOKENS.match(/--focus-ring-contrast:\s*(#[0-9a-fA-F]{6})/)[1];
    expect(ratio(contrastRing, HC_BASE)).toBeGreaterThanOrEqual(7);
  });
});

// ── 3. Adoption ─────────────────────────────────────────────────────────────

describe('every component adopts the shared control family', () => {
  // A COMPONENT is a file that registers a custom element. The others under
  // gui/components/ are shared fragments the components import —
  // ph-console-styles.js carries the control family itself, ph-scope-chrome.js
  // the corner readouts every radar draws — and a fragment has no shadow root
  // to adopt anything into. Asked by what they ARE rather than named in an
  // exemption list, so a third fragment does not have to remember to add
  // itself here.
  const components = componentFiles()
    // Registers a custom element either the old way (customElements.define)
    // or, since #1236's PhElement migration, via a phDefine('tag', Ctor) call
    // — both mark a file as a component rather than a shared fragment. The
    // quote after the paren is what distinguishes an actual call from
    // ph-element.js's own `export function phDefine(tag, ctor) {` declaration.
    .filter((f) => /customElements\.define|phDefine\(['"]/.test(fs.readFileSync(f, 'utf8')));

  it('finds the components to check', () => {
    expect(components.length).toBeGreaterThanOrEqual(30);
  });

  for (const file of components) {
    it(`${rel(file)} adopts it`, () => {
      const source = fs.readFileSync(file, 'utf8');
      if (!/attachShadow/.test(source)) {
        // A component that shares its base class's shadow root (ph-courier-radar
        // extends ph-tactical-radar) adopts through it. Assert that, rather
        // than exempting the file and hoping.
        expect(source).toMatch(/extends\s+Ph[A-Za-z]+/);
        return;
      }
      expect(source).toMatch(/phAdoptConsoleStyles\(this\.shadowRoot\)/);
    });
  }

  it('the console document adopts it too, so light-DOM markup matches', () => {
    // Shadow DOM blocks class rules, which is why console.css used to answer
    // `class="btn"` with a second, differently scaled copy of the design.
    const core = fs.readFileSync(path.join(GUI, 'console-core.js'), 'utf8');
    expect(core).toMatch(/phAdoptConsoleStyles\(document\)/);
  });

  it('the control family has one definition, not one per side of the boundary', () => {
    const consoleCss = readStripped(path.join(GUI, 'console.css'));
    // console.css must no longer carry its own button geometry.
    expect(consoleCss).not.toMatch(/\.btn\s*\{/);
    expect(consoleCss).not.toMatch(/\.chip\s*\{/);
  });

  it('the family is one design with size variants, not several designs', () => {
    const family = fs.readFileSync(path.join(GUI, 'components', 'ph-console-styles.js'), 'utf8');
    for (const variant of ['.btn--sm', '.btn--md', '.btn--lg']) {
      expect(family).toContain(variant);
    }
    // Each variant sets tokens; the geometry is written once and reads them.
    expect(family).toMatch(/--btn-h:\s*var\(--control-h-lg\)/);
    expect(family).toMatch(/height:\s*var\(--btn-h\)/);
  });
});

describe('no custom property is defined in terms of itself', () => {
  // A cycle is guaranteed-invalid, and it fails silently: the property resolves
  // to nothing and whatever read it falls back to its initial value. The
  // control family shipped `--btn-cham: calc(var(--btn-cham) - 0.04rem)` on the
  // recessed body of every button, which computed the body's `clip-path` to
  // `none` — the chamfered silhouette simply stopped being cut, and nothing
  // said so. Caught in the browser, pinned here.
  for (const file of [...SURFACES, TOKENS_CSS]) {
    it(`${rel(file)} has no self-referencing custom property`, () => {
      const source = readStripped(file);
      const decl = /(--[a-z0-9-]+)\s*:\s*([^;}]*)/gi;
      const cycles = [];
      let m;
      while ((m = decl.exec(source)) !== null) {
        const [, name, value] = m;
        // A self-reference is `var(--x)` — the SAME property, closed by a
        // delimiter (`)` or the `,` before a fallback). Terminating on a bare
        // `\b` would misread `var(--focus-ring-contrast)` as a cycle of
        // `--focus-ring`, since a hyphen is a word boundary; the high-contrast
        // palette legitimately aliases `--focus-ring: var(--focus-ring-contrast)`
        // (issue #1171). The documented real cycle — `var(--btn-cham)` closed by
        // `)` — is still caught.
        if (new RegExp(`var\\(\\s*${name}\\s*[,)]`).test(value)) cycles.push(`${name}: ${value.trim()}`);
      }
      expect(cycles).toEqual([]);
    });
  }
});

// ── 4. No literals outside the token file ───────────────────────────────────

const SURFACES = [
  ...componentFiles(),
  ...consoleDocuments(),
  path.join(GUI, 'console.css'),
  path.join(REPO_ROOT, 'client.html'),
];

describe('no stylesheet hardcodes a colour', () => {
  for (const file of SURFACES) {
    const name = rel(file);
    if (EXEMPT.has(name)) continue;
    it(`${name} names tokens instead of colours`, () => {
      const found = colourLiterals(readStripped(file));
      expect([...new Set(found)]).toEqual([]);
    });
  }
});

describe('no stylesheet hardcodes a type size', () => {
  for (const file of SURFACES) {
    const name = rel(file);
    if (EXEMPT.has(name)) continue;
    it(`${name} sizes text from the ramp`, () => {
      const found = fontSizeLiterals(readStripped(file));
      expect([...new Set(found)]).toEqual([]);
    });
  }
});
