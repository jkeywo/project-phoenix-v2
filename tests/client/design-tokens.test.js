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
 *
 * A whole file is a blunt exemption, so rule 2 and rule 3 also take a per-VALUE
 * one: KNOWN_LITERALS lists the literals a named surface is still allowed to
 * carry, and can only shrink. See its comment for what that buys.
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
 *
 * This excuses a whole FILE. KNOWN_LITERALS below excuses named VALUES inside
 * one, which is what a surface part-way through a migration wants: the rule
 * stays live over every other value in it.
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

  // --surface-base, the reference background. Graphite since issue #1357
  // (PRD #1355), where it moved from the navy #0a1028 to the splash
  // deliverable's page colour. It is DARKER in luminance terms (0.0037 against
  // 0.0059), so every accent's ratio climbed rather than fell.
  const BASE = '#0a0c10';

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

  // --surface-panel is a LIFT off the reference background since issue #1357,
  // and an enforced text rung is drawn on it — a console's content column is
  // filled with it, and gui/console.css's 0.6rem footers sit in that column.
  // --ink-faint was derived against it for exactly that reason; --fire was the
  // rung that reached AA on --surface-base (4.70) and failed on the panel
  // (4.32) while this suite stayed green, so the panel is asserted too.
  const PANEL = '#14171c';

  it('measures against the panel a console fills its content column with', () => {
    expect(hex('--surface-panel')).toBe(PANEL);
  });

  for (const name of ['--ink', '--ink-dim', '--ink-faint', '--tactical', '--fire',
    '--fire-bright', '--loaded', '--reloading', '--cyan', '--gold', '--signal',
    '--sky', '--science']) {
    it(`${name} clears 4.5:1 as text on --surface-panel`, () => {
      expect(ratio(hex(name), PANEL)).toBeGreaterThanOrEqual(4.5);
    });
  }

  // WCAG 1.4.11 measures a control's boundary against the colour ADJACENT to
  // it — the fill it encloses — not against a reference background. So the
  // control boundary is a LADDER, and the guarantee has to be enforced on the
  // surfaces each rung is actually claimed to carry. Asserting only
  // `--edge-control` against `--surface-base` was green while --edge-control
  // sat at 2.94 on --surface-card (gui/host-lobby.css, gui/host-scenarios.css)
  // and 2.20 on --surface-panel-up (server.html's GM block,
  // gui/components/ph-ship-picker.js). This table is the Edges block of
  // gui/tokens.css restated as a test; the two must move together.
  const CONTROL_LADDER = {
    '--edge-control': ['--surface-void', '--surface-abyss', '--surface-deep', '--surface-base'],
    '--edge-strong': ['--surface-card', '--surface-raised', '--surface-panel',
      '--surface-high', '--surface-lift'],
    '--edge-bright': ['--surface-panel-up'],
  };

  for (const [edge, surfaces] of Object.entries(CONTROL_LADDER)) {
    for (const surface of surfaces) {
      it(`${edge} clears 1.4.11's 3:1 on ${surface}, a surface it bounds controls over`, () => {
        const e = hex(edge);
        const s = hex(surface);
        expect(e).not.toBeNull();
        expect(s).not.toBeNull();
        expect(ratio(e, s)).toBeGreaterThanOrEqual(3);
      });
    }
  }

  it('keeps the ladder ordered, so a lighter fill always takes a lighter rung', () => {
    // If a rung ever overtook the one above it the table would still pass while
    // meaning nothing — "step up for a lighter fill" has to stay true.
    const rungs = ['--edge-faint', '--edge', '--edge-control', '--edge-strong', '--edge-bright'];
    const ratios = rungs.map((n) => ratio(hex(n), BASE));
    for (let i = 1; i < ratios.length; i += 1) {
      expect(ratios[i]).toBeGreaterThan(ratios[i - 1]);
    }
  });

  it('documents why the content edges are exempt rather than leaving it silent', () => {
    // `--edge` divides content — a panel column's border. The column is
    // legible without it, so it carries no information a player must perceive,
    // and it stays at the authored graphite. That reasoning has to be written
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

/**
 * The surfaces this rule is enforced over.
 *
 * All four host surfaces join the list in issue #1356. gui/host-qr.css joins
 * clean: the join panel was already written against the vocabulary, so listing
 * it is what stops it drifting back out. Its three siblings — gui/host-lobby.css,
 * gui/host-scenarios.css and server.html's inline <style> — were lifted verbatim
 * out of the host page and still carried the pre-graphite lobby palette, so they
 * joined with an allowlist instead. Issue #1357 retinted the vocabulary itself,
 * which gave the first two a rung to name for every value they were holding raw,
 * and their allowlist entries are empty. KNOWN_LITERALS below is what remains,
 * and says why it is the useful shape.
 *
 * gui/host-landing.css joins in the slice that CREATED it (issue #1360), with
 * no allowlist entry, and that timing is the point rather than tidiness. #1357
 * had just emptied the two entries above; a new host stylesheet allowed to
 * arrive unlisted would have switched the rule off again on the newest surface
 * in the fleet, which is the state #1356 filed this list to end. A sheet
 * written against the vocabulary from its first line costs nothing to enforce,
 * and enforcing it from its first line is what stops it acquiring the raw
 * palette its three siblings each had to be walked back out of.
 */
const SURFACES = [
  ...componentFiles(),
  ...consoleDocuments(),
  path.join(GUI, 'console.css'),
  path.join(GUI, 'host-landing.css'),
  path.join(GUI, 'host-lobby.css'),
  path.join(GUI, 'host-qr.css'),
  path.join(GUI, 'host-scenarios.css'),
  // gui/native-settings.css joins in the slice that created it (issue #1367),
  // by the same rule gui/host-landing.css did in #1360: a host stylesheet that
  // arrives unlisted switches this rule off on the newest surface in the fleet,
  // and a sheet written against the vocabulary from its first line costs
  // nothing to enforce. No allowlist entry, and there must never be one.
  path.join(GUI, 'native-settings.css'),
  // gui/game-over.css joins in the slice that created it, by the same rule:
  // the one sheet the phone, the Viewscreen and the native HUD share for the
  // ending, written against the vocabulary from its first line. No allowlist
  // entry, and there must never be one.
  path.join(GUI, 'game-over.css'),
  path.join(REPO_ROOT, 'client.html'),
  path.join(REPO_ROOT, 'server.html'),
];

/**
 * The literals a surface is still allowed to carry, named one by one.
 *
 * A RATCHET, not an exemption. The rule stays live over every OTHER value in
 * these files, which is the whole point: plant a new hex in gui/host-lobby.css
 * and this suite says so on the next run. The alternative considered — leave
 * the three out of SURFACES until the palette lands — would have left the rule
 * switched off exactly where #1356 did its work.
 *
 * What was listed is the pre-graphite lobby palette: #cce ink on #0e0e14 cards
 * with #2a8a96 claimed borders, around a hundred distinct values between the
 * three, and gui/tokens.css named almost none of them. #1356 swapped every
 * literal that DID have an exact token — --signal, --surface-void, the
 * --rgb-* triplets, the type rungs — and stopped there, because rounding what
 * was left onto the nearest rung would have been a retint smuggled in under a
 * slice whose whole claim was that nothing moves.
 *
 * #1357 is where that retint was actually decided, in the one file the
 * vocabulary lives in — and with a graphite rung for every role the lobby was
 * naming in raw hex, both stylesheet entries went to `[]`. They are kept as
 * empty objects rather than deleted so the ratchet's end state is visible in
 * the file that enforced it: these two surfaces are now held at zero, exactly
 * like the ones that never needed an entry.
 *
 * The list can only SHRINK: a value that leaves a file must leave here too,
 * because the assertions below fail on a STALE entry as well as on a new
 * literal. That is what made #1355 a list this suite watched empty out, rather
 * than a SURFACES edit somebody had to remember to make.
 *
 * server.html needs more than the retint before its entry reaches `[]`.
 * Issue #1449 finishes the inline <style> retint under explicit permission to
 * change appearance. Its zero-literal check below is separate from this
 * allowance, which now covers only JavaScript and style attributes.
 * css-scan.js reads the WHOLE file, not the <style> block — there is no lexer
 * that will hand back "the stylesheet" — and this page carries colour well
 * outside it: the `style=` attributes on the GM join panel and the
 * asset-loading overlay. Those have to move into the stylesheet layer on their
 * own account. The game-over overlay's share (its JS accent table and its
 * inline styles) left with gui/game-over.css, which holds the ending at zero.
 *
 * To regenerate an entry, ask the scanner that enforces it:
 *
 *   node --input-type=module -e "import { readStripped, colourLiterals,  *     fontSizeLiterals } from './tests/client/css-scan.js'; const s =  *     readStripped('server.html'); console.log([...new Set(colourLiterals(s))],  *     [...new Set(fontSizeLiterals(s))])"
 */
const KNOWN_LITERALS = {
  "gui/host-lobby.css": { colours: [], sizes: [] },
  "gui/host-scenarios.css": { colours: [], sizes: [] },
  "server.html": {
    "colours": [
      "#0d0000",
      "#ff8888",
      "#ff4444",
      "#1a0000",
      "#550000",
      "#888",
      "#6ea4c8",
      "#d8edff",
      "#000",
      "#aae",
      "#fa4",
      "#8af",
      "#4c4",
      "#f44",
      "rgba(4,12,22 …)",
      "rgba(0,0,0 …)"
    ],
    "sizes": [
      "font-size: 0.85rem",
      "font-size: 1.1rem",
      "font-size: 0.9rem",
      "font-size: 13px",
      "font-size: 12px",
      "font-size: 16px"
    ]
  },
};

/** The listed literals for a surface, or none — an unlisted file allows zero. */
const known = (name, kind) => KNOWN_LITERALS[name]?.[kind] ?? [];

describe('the host inline styles use tokens without legacy exceptions', () => {
  const styles = [...readStripped(path.join(REPO_ROOT, 'server.html'))
    .matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map(match => match[1]);
  it('checks a real stylesheet and permits no colour or type-size literals', () => {
    expect(styles.length).toBeGreaterThan(0);
    for (const style of styles) {
      expect(colourLiterals(style)).toEqual([]);
      expect(fontSizeLiterals(style)).toEqual([]);
    }
  });
});

describe('no stylesheet hardcodes a colour', () => {
  for (const file of SURFACES) {
    const name = rel(file);
    if (EXEMPT.has(name)) continue;
    it(`${name} names tokens instead of colours`, () => {
      const found = [...new Set(colourLiterals(readStripped(file)))];
      const allowed = known(name, 'colours');
      expect(found.filter((v) => !allowed.includes(v)),
        `${name} grew a colour literal that is not in KNOWN_LITERALS`).toEqual([]);
      expect(allowed.filter((v) => !found.includes(v)),
        `${name} no longer has these; drop them from KNOWN_LITERALS`).toEqual([]);
    });
  }
});

describe('no stylesheet hardcodes a type size', () => {
  for (const file of SURFACES) {
    const name = rel(file);
    if (EXEMPT.has(name)) continue;
    it(`${name} sizes text from the ramp`, () => {
      const found = [...new Set(fontSizeLiterals(readStripped(file)))];
      const allowed = known(name, 'sizes');
      expect(found.filter((v) => !allowed.includes(v)),
        `${name} grew a type-size literal that is not in KNOWN_LITERALS`).toEqual([]);
      expect(allowed.filter((v) => !found.includes(v)),
        `${name} no longer has these; drop them from KNOWN_LITERALS`).toEqual([]);
    });
  }
});
