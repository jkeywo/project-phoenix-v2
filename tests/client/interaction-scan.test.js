/**
 * tests/client/interaction-scan.test.js — the shared interaction scanner
 * (issue #1175), tested against small source fixtures.
 *
 * interaction-floors.test.js points this scanner at the real console surface;
 * this file proves the scanner itself classifies correctly — and, crucially,
 * that the floors it feeds are not vacuous. A floor that cannot fail proves
 * nothing, so the NEGATIVE fixtures below (an unnamed button, an unreachable
 * drag widget, a glyph with no aria-label, a control pulled from the tab order)
 * each show the scanner reporting the violation the real floor keys on.
 */
import { describe, it, expect } from 'vitest';
import {
  evaluateSurface, focusFamilyAdoption, componentFiles, rel,
} from './interaction-scan.js';
import fs from 'node:fs';

const read = (name) => fs.readFileSync(componentFiles().find((f) => rel(f) === name), 'utf8');

// ── Conformant fixtures — the shapes the tracer uses ────────────────────────

const CONFORMANT_TOOLBAR = `
class PhX extends HTMLElement {
  constructor() { super(); this.attachShadow({ mode: 'open' }); phAdoptConsoleStyles(this.shadowRoot); }
  connectedCallback() {
    this.setAttribute('role', 'toolbar');
    this.setAttribute('aria-label', t('component.x.title'));
    installRovingTabindex(this, { getItems: () => [] });
    const btn = document.createElement('button');
    btn.innerHTML = '<span class="label">' + t('console.common.fire') + '</span>';
  }
}
customElements.define('ph-x', PhX);
`;

const CANVAS_GROUP = `
class PhG extends HTMLElement {
  constructor() { super(); this.attachShadow({ mode: 'open' }); phAdoptConsoleStyles(this.shadowRoot); }
  connectedCallback() {
    this.setAttribute('role', 'group');
    this.setAttribute('aria-label', t('component.g.label'));
    this.setAttribute('tabindex', '0');
    this.addEventListener('keydown', () => {});
  }
}
customElements.define('ph-g', PhG);
`;

const GLYPH_WITH_ARIA = `
const b = document.createElement('button');
b.innerHTML = '<span class="lbl">−</span>';
b.setAttribute('aria-label', t('component.x.decrease'));
b.addEventListener('click', () => {});
`;

const DESCENDANT_NAME = `
const row = document.createElement('button');
row.innerHTML = '<span class="name"></span>';
row.querySelector('.name').textContent = d.name;
row.addEventListener('click', () => {});
`;

const RUNTIME_NAME = `
class PhA extends HTMLElement {
  constructor() { super(); this.attachShadow({ mode: 'open' }); phAdoptConsoleStyles(this.shadowRoot);
    this.shadowRoot.innerHTML = '<button id="go" type="button"></button>'; }
  connectedCallback() { this.shadowRoot.getElementById('go').textContent = t('x'); }
}
customElements.define('ph-a', PhA);
`;

const REACHABLE_WITH_ROVING = `
this.setAttribute('role', 'toolbar');
this.setAttribute('aria-label', t('x'));
installRovingTabindex(this, {});
document.body.innerHTML = '<button>Go</button><button tabindex="-1">Extra</button>';
`;

const PURE_READOUT = `
class PhR extends HTMLElement {
  constructor() { super(); this.attachShadow({ mode: 'open' }); phAdoptConsoleStyles(this.shadowRoot); }
}
customElements.define('ph-r', PhR);
`;

// ── Negative fixtures — each isolates one floor's failure ───────────────────

const UNNAMED_BUTTON = '<button type="button"></button>';

const GLYPH_NO_ARIA = `
const b = document.createElement('button');
b.innerHTML = '<span class="lbl">−</span>';
b.addEventListener('click', () => {});
`;

const UNFOCUSABLE_WIDGET = `
class PhW extends HTMLElement {
  constructor() { super(); this.attachShadow({ mode: 'open' }); phAdoptConsoleStyles(this.shadowRoot); }
  connectedCallback() {
    const well = this.shadowRoot.querySelector('.well');
    well.addEventListener('pointerdown', () => {});
  }
}
customElements.define('ph-w', PhW);
`;

const STRANDED_CONTROL = '<button>Go</button><button tabindex="-1">Extra</button>';

const evalC = (src) => evaluateSurface(src, { isDocument: false });

describe('the scanner passes conformant shapes', () => {
  it('a roving toolbar with a role, name and named button is conformant', () => {
    const r = evalC(CONFORMANT_TOOLBAR);
    expect(r.hasSurface).toBe(true);
    expect(r.floors).toEqual({ focusability: [], reachability: [], naming: [] });
  });

  it('a canvas scope group (host tabindex + role + name) is conformant', () => {
    const r = evalC(CANVAS_GROUP);
    expect(r.compositeHost).toBe(true);
    expect(r.focusTargets).toBe(1);       // the host itself is the tab stop
    expect(r.conformant).toBe(true);
  });

  it('a glyph stepper WITH an aria-label is named', () => {
    expect(evalC(GLYPH_WITH_ARIA).floors.naming).toEqual([]);
  });

  it('a button named through a descendant it fills is named', () => {
    expect(evalC(DESCENDANT_NAME).floors.naming).toEqual([]);
  });

  it('a markup control named at runtime by its id is named', () => {
    expect(evalC(RUNTIME_NAME).floors.naming).toEqual([]);
  });

  it('a tabindex="-1" control is reachable when roving is installed', () => {
    expect(evalC(REACHABLE_WITH_ROVING).floors.reachability).toEqual([]);
  });

  it('a pure readout with no controls has no interactive surface', () => {
    expect(evalC(PURE_READOUT).hasSurface).toBe(false);
  });
});

describe('the scanner fails a genuine violation (the floors are not vacuous)', () => {
  it('an empty <button> fails the accessible-name floor', () => {
    const r = evalC(UNNAMED_BUTTON);
    expect(r.hasSurface).toBe(true);
    expect(r.floors.naming.length).toBeGreaterThan(0);
    expect(r.floors.naming.join(' ')).toMatch(/no accessible name/);
  });

  it('a glyph-only <button> with no aria-label fails the accessible-name floor', () => {
    const r = evalC(GLYPH_NO_ARIA);
    expect(r.floors.naming.join(' ')).toMatch(/glyph-only/);
  });

  it('a pointerdown widget with nothing focusable fails focusability, reachability AND naming', () => {
    const r = evalC(UNFOCUSABLE_WIDGET);
    expect(r.hasSurface).toBe(true);
    expect(r.focusTargets).toBe(0);
    expect(r.floors.focusability.length).toBeGreaterThan(0);
    expect(r.floors.reachability.length).toBeGreaterThan(0);
    expect(r.floors.naming.join(' ')).toMatch(/custom interactive widget/);
  });

  it('a control pulled from the tab order with no roving fails reachability', () => {
    const r = evalC(STRANDED_CONTROL);
    expect(r.focusTargets).toBe(1);                 // the sibling is still reachable
    expect(r.floors.reachability.join(' ')).toMatch(/tabindex="-1"/);
  });
});

describe('focus-family adoption', () => {
  it('a component that calls phAdoptConsoleStyles adopts', () => {
    expect(focusFamilyAdoption(CONFORMANT_TOOLBAR).ok).toBe(true);
  });

  it('a component that only extends a sibling inherits adoption', () => {
    const src = 'class PhY extends PhTacticalRadar {}\ncustomElements.define("ph-y", PhY);';
    const a = focusFamilyAdoption(src);
    expect(a.definesElement).toBe(true);
    expect(a.adopts).toBe(false);
    expect(a.ok).toBe(true);
  });

  it('a component that neither adopts nor extends a sibling fails', () => {
    const src = 'class PhZ extends HTMLElement {}\ncustomElements.define("ph-z", PhZ);';
    expect(focusFamilyAdoption(src).ok).toBe(false);
  });

  it('a plain helper module (no custom element) is not a component', () => {
    expect(focusFamilyAdoption('export function helper() {}').definesElement).toBe(false);
  });
});

describe('the scanner agrees with the real surface', () => {
  it('the tracer phasers component is conformant', () => {
    expect(evalC(read('gui/components/ph-phasers-controls.js')).conformant).toBe(true);
  });

  // There is no longer a "real debt component (…) is not conformant" example here.
  // It walked ph-power-controls (#1170→#1177) then the ship picker (#1177→#1178) as
  // each family converted; #1178 was the final sweep, so no real component fails a
  // floor any more. The floors-can-fail guarantee lives entirely in the synthetic
  // negative fixtures above ('the scanner fails a genuine violation'), which isolate
  // each floor's failure without leaning on a real component that will later be fixed.
});
