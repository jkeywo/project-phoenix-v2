import { describe, it, expect, beforeEach } from 'vitest';
import { setTable } from '../../gui/strings.js';
import {
  renderManual,
  renderSection,
  formatMetricValue,
  ratingCaption,
} from '../../gui/manual-panel.js';

// ── Minimal DOM stub (same shape as settings-panel.test.js) ─────────────────

function makeEl(doc, tag) {
  const listeners = {};
  const el = {
    ownerDocument: doc,
    tagName: String(tag).toUpperCase(),
    children: [],
    attributes: {},
    classList: new Set(),
    _id: '',
    hidden: false,
    textContent: '',
    type: '',
    title: '',
    get id() { return this._id; },
    set id(v) { this._id = v; if (v) doc._byId[v] = this; },
    set innerHTML(_v) { this.children = []; },
    setAttribute(k, v) { this.attributes[k] = String(v); },
    getAttribute(k) { return this.attributes[k]; },
    appendChild(child) { this.children.push(child); child.parentNode = this; return child; },
    addEventListener(type, fn) { (listeners[type] = listeners[type] || []).push(fn); },
    dispatch(type, ev) {
      (listeners[type] || []).forEach((fn) => fn(ev || { preventDefault() {}, stopPropagation() {}, target: el }));
    },
    click() { this.dispatch('click'); },
    querySelector() { return null; },
    querySelectorAll() { return []; },
  };
  el.classList.add = (c) => Set.prototype.add.call(el.classList, c);
  el.classList.remove = (c) => Set.prototype.delete.call(el.classList, c);
  el.classList.contains = (c) => Set.prototype.has.call(el.classList, c);
  Object.defineProperty(el, 'className', {
    get() { return Array.from(el.classList).join(' '); },
    set(v) { el.classList.clear(); String(v).split(/\s+/).filter(Boolean).forEach((c) => el.classList.add(c)); },
  });
  return el;
}

function makeDoc() {
  const doc = {
    _byId: {},
    createElement(tag) { return makeEl(this, tag); },
    getElementById(id) { return this._byId[id] || null; },
  };
  doc.body = makeEl(doc, 'body');
  doc.documentElement = makeEl(doc, 'html');
  return doc;
}

/** Collect all textContent in a subtree (depth-first). */
function allText(el) {
  let out = el.textContent ? [el.textContent] : [];
  for (const c of el.children) out = out.concat(allText(c));
  return out;
}

// A fixture string table so labels resolve to real English (proving the panel
// goes through t() rather than hardcoding text).
const TABLE = new Map([
  ['station.captain.name', 'Captain'],
  ['station.science.name', 'Science'],
  ['manual.heading', 'SHIP MANUAL'],
  ['manual.title', 'Ship Manual'],
  ['manual.empty', 'Ship manual unavailable'],
  ['manual.section.shields', 'Shields'],
  ['manual.shields.max_hp', 'Base strength: {value} HP per arc'],
  ['manual.shields.regen', 'Regeneration: {value} HP/s per arc'],
  ['manual.shields.arcs', 'Shield arcs: {value}'],
  ['manual.automated_systems', 'AI automation by rating'],
  ['manual.rating.none', 'none'],
  ['station.rating.std.name', 'STANDARD'],
  ['station.rating.backfill.name', 'BACKFILL (AI)'],
  ['manual.section.helm_thrust', 'Helm'],
  ['manual.helm_thrust.max_speed', 'Max speed: {value}'],
  ['manual.helm_thrust.movement_mode', 'Movement mode: {value}'],
  ['manual.helm_thrust.movement_mode.bounded', 'Bounded vertical'],
]);

function fixtureManual() {
  return {
    stations: [
      {
        station_id: 'captain',
        overview: 'You command the bridge.',
        sections: [],
      },
      {
        station_id: 'science',
        overview: 'Sensors and shields.',
        sections: [
          {
            kind: 'shields',
            metrics: [
              { code: 'max_hp', value: 100 },
              { code: 'regen', value: 2 },
              { code: 'arcs', value: 4 },
            ],
            automation: [
              { rating: 'Std', automated_systems: [] },
              { rating: 'Backfill', automated_systems: ['shields-system', 'shield-arc-fore'] },
            ],
          },
        ],
      },
    ],
  };
}

describe('manual-panel formatting helpers', () => {
  beforeEach(() => setTable(TABLE));

  it('formats whole numbers without a decimal tail', () => {
    expect(formatMetricValue(100)).toBe('100');
    expect(formatMetricValue(4)).toBe('4');
  });

  it('formats fractional numbers to two decimals', () => {
    expect(formatMetricValue(2.5)).toBe('2.5');
  });

  it('resolves known rating captions via the string table', () => {
    expect(ratingCaption('Std')).toBe('STANDARD');
    expect(ratingCaption('Backfill')).toBe('BACKFILL (AI)');
  });
});

describe('renderManual', () => {
  beforeEach(() => setTable(TABLE));

  it('renders one tab per authored station', () => {
    const doc = makeDoc();
    const root = doc.createElement('div');
    const count = renderManual(root, fixtureManual());
    expect(count).toBe(2);
    const tabs = root.children[0].children;
    expect(tabs.length).toBe(2);
    expect(tabs[0].textContent).toBe('Captain');
    expect(tabs[1].textContent).toBe('Science');
  });

  it('shows the active station overview and its generated section labels via t()', () => {
    const doc = makeDoc();
    const root = doc.createElement('div');
    renderManual(root, fixtureManual(), 1); // Science tab active
    const text = allText(root).join('\n');
    expect(text).toContain('Sensors and shields.');
    // Labels come from the string table, with the numeric value interpolated.
    expect(text).toContain('Base strength: 100 HP per arc');
    expect(text).toContain('Regeneration: 2 HP/s per arc');
    expect(text).toContain('Shield arcs: 4');
    // Rating→AI automation is rendered.
    expect(text).toContain('AI automation by rating');
    expect(text).toContain('BACKFILL (AI)');
    expect(text).toContain('shields-system, shield-arc-fore');
  });

  it('renders an overview-only station with no section labels', () => {
    const doc = makeDoc();
    const root = doc.createElement('div');
    renderManual(root, fixtureManual(), 0); // Captain tab active
    const text = allText(root).join('\n');
    expect(text).toContain('You command the bridge.');
    expect(text).not.toContain('Shields');
  });

  it('shows an empty message when there is no manual', () => {
    const doc = makeDoc();
    const root = doc.createElement('div');
    const count = renderManual(root, null);
    expect(count).toBe(0);
    expect(allText(root).join('\n')).toContain('Ship manual unavailable');
  });

  it('renders a non-numeric capability via t() with a resolved value_code', () => {
    // The Helm movement mode is carried as a machine value_code and rendered
    // through `manual.<kind>.<code>.<value_code>` interpolated into the label.
    const doc = makeDoc();
    const section = {
      kind: 'helm_thrust',
      metrics: [{ code: 'max_speed', value: 10 }],
      capabilities: [{ code: 'movement_mode', value_code: 'bounded' }],
      automation: [],
    };
    const el = renderSection(doc, section);
    const text = allText(el).join('\n');
    expect(text).toContain('Max speed: 10');
    expect(text).toContain('Movement mode: Bounded vertical');
  });

  it('is read-only: a section builds no interactive controls that send commands', () => {
    // renderSection must never wire a network send; it only builds display DOM.
    const doc = makeDoc();
    const section = fixtureManual().stations[1].sections[0];
    const el = renderSection(doc, section);
    // No <button> elements are produced by a section (tabs are the only
    // buttons, and they only switch the visible panel).
    const hasButton = (node) =>
      node.tagName === 'BUTTON' || node.children.some(hasButton);
    expect(hasButton(el)).toBe(false);
  });
});
