import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach } from 'vitest';
import {
  helpSections,
  hasHelp,
  openHelp,
  closeHelp,
  isHelpOpen,
  createHelpButton,
  mountHelp,
  renderInlineHelp,
} from '../../gui/help-panel.js';

// ── helpSections static text (mirrors elements.rs help_sections) ─────────────

describe('helpSections', () => {
  // Sections resolve through the string table (help.<station>.<n>.heading/body)
  // so assertions go through t() — the id, not the English, is the contract.
  it('returns the captain tuples', () => {
    expect(helpSections('captain')).toEqual([
      [t('help.captain.0.heading'), t('help.captain.0.body')],
      [t('help.captain.1.heading'), t('help.captain.1.body')],
      [t('help.captain.2.heading'), t('help.captain.2.body')],
    ]);
  });

  it('returns the helm tuples (5 sections incl. the 10× burst)', () => {
    const h = helpSections('helm');
    expect(h).toHaveLength(5);
    expect(h[0]).toEqual([t('help.helm.0.heading'), t('help.helm.0.body')]);
    expect(h[2]).toEqual([t('help.helm.2.heading'), t('help.helm.2.body')]);
  });

  it('documents the impulse and boost bindings on helm', () => {
    const byTitle = Object.fromEntries(helpSections('helm'));
    expect(byTitle[t('help.helm.2.heading')]).toContain('Ctrl');
    expect(byTitle[t('help.helm.2.heading')]).toContain('gamepad B');
    expect(byTitle[t('help.helm.3.heading')]).toContain('Hold Shift');
    expect(byTitle[t('help.helm.3.heading')]).toContain('gamepad A');
  });

  it('returns the tactical tuples', () => {
    expect(helpSections('tactical')).toEqual([
      [t('help.tactical.0.heading'), t('help.tactical.0.body')],
      [t('help.tactical.1.heading'), t('help.tactical.1.body')],
      [t('help.tactical.2.heading'), t('help.tactical.2.body')],
      [t('help.tactical.3.heading'), t('help.tactical.3.body')],
    ]);
  });

  it('covers all console keys for all ship types', () => {
    for (const key of ['captain', 'helm', 'tactical', 'repair', 'power',
                        'shields', 'sensors', 'navigation', 'comms',
                        'engineering', 'science']) {
      expect(hasHelp(key)).toBe(true);
      expect(helpSections(key).length).toBeGreaterThan(0);
    }
  });

  it('preserves the literal × glyph in helm Impulse text', () => {
    const impulse = helpSections('helm').find(([h]) => h === t('help.helm.2.heading'));
    expect(impulse[1]).toContain('×');
  });

  it('returns an empty array for an unknown key, and hasHelp is false', () => {
    expect(helpSections('Nope')).toEqual([]);
    expect(hasHelp('Nope')).toBe(false);
  });
});

// ── Minimal DOM stub (node env has no document) ──────────────────────────────

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
    dispatch(type, ev) { (listeners[type] || []).forEach((fn) => fn(ev || { preventDefault() {}, stopPropagation() {} })); },
    click() { this.dispatch('click'); },
    querySelector() { return null; },
  };
  // Mirror classList add/remove onto a className-ish surface for assertions.
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
    _query: {},
    readyState: 'complete',
    createElement(tag) { return makeEl(this, tag); },
    getElementById(id) { return this._byId[id] || null; },
    querySelector(sel) { return this._query[sel] || null; },
  };
  doc.body = makeEl(doc, 'body');
  doc.documentElement = makeEl(doc, 'html');
  return doc;
}

function findOverlay(doc) {
  return doc.getElementById('help-overlay');
}

// ── Modal open / close / dismiss ─────────────────────────────────────────────

describe('help modal open/close', () => {
  let doc;
  beforeEach(() => { doc = makeDoc(); });

  it('openHelp creates the overlay, fills it, and reveals it', () => {
    expect(isHelpOpen(doc)).toBe(false);
    openHelp('repair', doc);

    const overlay = findOverlay(doc);
    expect(overlay).not.toBeNull();
    expect(overlay.hidden).toBe(false);
    expect(overlay.getAttribute('aria-hidden')).toBe('false');
    expect(isHelpOpen(doc)).toBe(true);

    // Heading + one section per Repair tuple.
    const heading = overlay.children.find((c) => c.className === 'help-heading');
    expect(heading.textContent).toBe(t('help.modal_heading'));
    const sections = overlay.children.find((c) => c.className === 'help-sections');
    expect(sections.children).toHaveLength(helpSections('repair').length);
  });

  it('closeHelp hides the overlay', () => {
    openHelp('helm', doc);
    expect(isHelpOpen(doc)).toBe(true);
    closeHelp(doc);
    expect(isHelpOpen(doc)).toBe(false);
    expect(findOverlay(doc).hidden).toBe(true);
    expect(findOverlay(doc).getAttribute('aria-hidden')).toBe('true');
  });

  it('clicking the overlay dismisses it', () => {
    openHelp('power', doc);
    findOverlay(doc).dispatch('click');
    expect(isHelpOpen(doc)).toBe(false);
  });

  it('re-uses a single overlay element across opens', () => {
    openHelp('helm', doc);
    const first = findOverlay(doc);
    closeHelp(doc);
    openHelp('comms', doc);
    expect(findOverlay(doc)).toBe(first);
    // Content swapped to the new panel.
    const sections = first.children.find((c) => c.className === 'help-sections');
    expect(sections.children).toHaveLength(helpSections('comms').length);
  });
});

// ── Trigger button ───────────────────────────────────────────────────────────

describe('createHelpButton', () => {
  it('builds a "?" button that opens the panel help on click', () => {
    const doc = makeDoc();
    const btn = createHelpButton('sensors', doc);
    expect(btn.textContent).toBe('?');
    expect(btn.className).toContain('help-btn');
    expect(isHelpOpen(doc)).toBe(false);
    btn.click();
    expect(isHelpOpen(doc)).toBe(true);
  });
});

describe('mountHelp', () => {
  it('appends a "?" button to the .frame and wires it', () => {
    const doc = makeDoc();
    const frame = makeEl(doc, 'div');
    doc._query['.frame'] = frame;

    const trigger = mountHelp('navigation', doc);
    expect(trigger).not.toBeNull();
    expect(frame.children).toContain(trigger);

    trigger.click();
    expect(isHelpOpen(doc)).toBe(true);
  });

  it('returns null for a console with no help text', () => {
    const doc = makeDoc();
    expect(mountHelp('Bogus', doc)).toBeNull();
  });

  it('uses an existing [data-help-button] element when present', () => {
    const doc = makeDoc();
    const existing = makeEl(doc, 'button');
    doc._query['[data-help-button]'] = existing;

    const trigger = mountHelp('shields', doc);
    expect(trigger).toBe(existing);
    trigger.click();
    expect(isHelpOpen(doc)).toBe(true);
  });
});

// ── renderInlineHelp ──────────────────────────────────────────────────────

describe('renderInlineHelp', () => {
  let doc;
  let root;

  function stubWindowLabel() {
    // Stub CONSOLE_LABEL on global window for label lookups. The keys here
    // mirror the wire-field PascalCase names that `renderInlineHelp` receives
    // in production (from `player.consoles`); the values are the labels to
    // show. Post issue #618 the caller may also pass lowercase station ids —
    // add both so the tests exercise both spellings.
    if (typeof globalThis !== 'undefined') {
      globalThis.CONSOLE_LABEL = {
        Helm: 'Helm', Tactical: 'Tactical', Repair: 'Repair',
        helm: 'Helm', tactical: 'Tactical', repair: 'Repair',
      };
    }
  }

  beforeEach(() => {
    doc = makeDoc();
    root = doc.createElement('div');
    stubWindowLabel();
  });

  it('renders help sections for a single console', () => {
    renderInlineHelp(root, ['helm'], doc);
    // Root should have 1 help-console-group child
    const groups = root.children.filter((c) => c.className === 'help-console-group');
    expect(groups).toHaveLength(1);
    const heading = groups[0].children.find((c) => c.className === 'help-console-heading');
    // In the browser the heading is looked up via `window.CONSOLE_LABEL`
    // (title-cased); in Node the fallback returns the raw station-id
    // argument. Both paths are exercised — assert on the Node fallback.
    expect(heading.textContent).toBe('helm');
    // One rendered child per helm help section.
    const sections = groups[0].children.find((c) => c.className === 'help-sections');
    expect(sections.children).toHaveLength(helpSections('helm').length);
  });

  it('renders help sections for multiple consoles', () => {
    renderInlineHelp(root, ['helm', 'repair'], doc);
    const groups = root.children.filter((c) => c.className === 'help-console-group');
    expect(groups).toHaveLength(2);
    // Helm first, Repair second (input order preserved)
    expect(groups[0].children.find((c) => c.className === 'help-console-heading').textContent).toBe('helm');
    expect(groups[1].children.find((c) => c.className === 'help-console-heading').textContent).toBe('repair');
  });

  it('skips consoles with no help text', () => {
    renderInlineHelp(root, ['helm', 'Bogus'], doc);
    const groups = root.children.filter((c) => c.className === 'help-console-group');
    expect(groups).toHaveLength(1);
  });

  it('is a no-op when root is null', () => {
    expect(() => renderInlineHelp(null, ['helm'])).not.toThrow();
  });

  it('is a no-op when consoles is empty', () => {
    renderInlineHelp(root, [], doc);
    expect(root.children).toHaveLength(0);
  });

  it('re-builds from scratch on each call (no stale content)', () => {
    renderInlineHelp(root, ['helm'], doc);
    expect(root.children).toHaveLength(1);
    renderInlineHelp(root, ['repair'], doc);
    expect(root.children).toHaveLength(1);
    // Now repair, not helm (see previous test for the raw-label rationale).
    const heading = root.children[0].children.find((c) => c.className === 'help-console-heading');
    expect(heading.textContent).toBe('repair');
  });

  it('renders correct section content for tactical', () => {
    renderInlineHelp(root, ['tactical'], doc);
    const sections = root.children[0].children.find((c) => c.className === 'help-sections');
    const titles = sections.children.map((s) => s.children.find((c) => c.className === 'help-section-title').textContent);
    expect(titles).toEqual([
      t('help.tactical.0.heading'),
      t('help.tactical.1.heading'),
      t('help.tactical.2.heading'),
      t('help.tactical.3.heading'),
    ]);
  });
});
