import { describe, it, expect } from 'vitest';
import { t } from '../../gui/strings.js';
import { helpSections, hasHelp, renderStationHelp } from '../../gui/help-panel.js';
import { createCaptainActionRegistry } from '../../gui/stations/captain-actions.js';

function makeEl(doc, tag) {
  const el = {
    ownerDocument: doc, tagName: String(tag).toUpperCase(), children: [], textContent: '',
    classList: new Set(), appendChild(child) { this.children.push(child); child.parentNode = this; return child; },
    set innerHTML(_value) { this.children = []; },
  };
  Object.defineProperty(el, 'className', {
    get() { return Array.from(el.classList).join(' '); },
    set(value) { el.classList = new Set(String(value).split(/\s+/).filter(Boolean)); },
  });
  return el;
}

function makeDoc() {
  const doc = { createElement(tag) { return makeEl(this, tag); } };
  doc.body = makeEl(doc, 'body');
  return doc;
}

describe('helpSections', () => {
  it('returns localized Captain help', () => {
    expect(helpSections('captain')).toEqual([
      [t('help.captain.0.heading'), t('help.captain.0.body')],
      [t('help.captain.1.heading'), t('help.captain.1.body')],
      [t('help.captain.2.heading'), t('help.captain.2.body')],
    ]);
  });

  it('covers all currently supported station ids', () => {
    for (const id of ['captain', 'helm', 'tactical', 'repair', 'power', 'shields', 'sensors', 'navigation', 'comms', 'engineering', 'science']) {
      expect(hasHelp(id)).toBe(true);
      expect(helpSections(id).length).toBeGreaterThan(0);
    }
    expect(hasHelp('bogus')).toBe(false);
    expect(helpSections('bogus')).toEqual([]);
  });
});

describe('renderStationHelp', () => {
  it('renders only the selected station help into the caller-owned Settings body', () => {
    const doc = makeDoc();
    const root = doc.createElement('div');
    expect(renderStationHelp(root, 'helm')).toBe(true);
    expect(root.children).toHaveLength(1);
    const sections = root.children[0].children.find((child) => child.className === 'station-help-sections');
    expect(sections.children).toHaveLength(helpSections('helm').length);
  });

  it('does not create standalone buttons or overlays for missing help', () => {
    const doc = makeDoc();
    const root = doc.createElement('div');
    expect(renderStationHelp(root, 'bogus')).toBe(false);
    expect(root.children).toHaveLength(0);
  });

  it('shows the Captain action metadata and both current binding slots', () => {
    const doc = makeDoc();
    const root = doc.createElement('div');
    const registry = createCaptainActionRegistry();
    registry.setBinding('captain.red-alert', 0, { code: 'KeyY', shiftKey: true });
    expect(renderStationHelp(root, 'captain', registry.list())).toBe(true);
    const flatten = (el) => [el.textContent]
      .concat(...(el.children || []).map(flatten));
    const text = flatten(root).join('\n');
    expect(text).toContain(t('help.bindings.heading'));
    expect(text).toContain(t('semantic_action.captain.red_alert.label'));
    expect(text).toContain(t('semantic_action.captain.red_alert.accessibility'));
    expect(text).toContain('Shift + Y');
    expect(text).toContain(t('input.binding.unassigned'));
  });
});
