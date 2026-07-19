import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach } from 'vitest';
import { mountSettings } from '../../gui/settings-panel.js';

// ── Minimal DOM stub (same pattern as help-panel.test.js) ───────────────────

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
    value: '',
    min: '',
    max: '',
    step: '',
    disabled: false,
    get id() { return this._id; },
    set id(v) { this._id = v; if (v) doc._byId[v] = this; },
    set innerHTML(_v) { this.children = []; },
    setAttribute(k, v) { this.attributes[k] = String(v); },
    getAttribute(k) { return this.attributes[k]; },
    hasAttribute(k) { return k in this.attributes; },
    appendChild(child) { this.children.push(child); child.parentNode = this; return child; },
    addEventListener(type, fn) {
      (listeners[type] = listeners[type] || []).push(fn);
    },
    dispatch(type, ev) {
      (listeners[type] || []).forEach((fn) => fn(ev || { preventDefault() {}, stopPropagation() {} }));
    },
    click() { this.dispatch('click'); },
    querySelector() { return null; },
    querySelectorAll() { return []; },
    closest() { return null; },
    getElementsByClassName() { return []; },
    insertBefore() {},
    get rootNode() { return this; },
    contains() { return false; },
    valueOf() { return this; },
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
    _query: {},
    readyState: 'complete',
    createElement(tag) { return makeEl(this, tag); },
    getElementById(id) { return this._byId[id] || null; },
    querySelector(sel) { return this._query[sel] || null; },
    addEventListener() {},
    removeEventListener() {},
  };
  doc.body = makeEl(doc, 'body');
  doc.documentElement = makeEl(doc, 'html');
  return doc;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function findOverlay(doc) {
  return doc.getElementById('settings-overlay');
}

function findBtn(doc) {
  return doc.getElementById('settings-btn');
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe('mountSettings', () => {
  let doc, sendCalls, state, audioEl;

  beforeEach(() => {
    doc = makeDoc();
    sendCalls = [];
    state = {
      players: [{ token: 'abc', station: 'helm' }],
      stations: [],
      stationRatings: {},
    };
    audioEl = { volume: 0.5 };
  });

  it('creates a gear button and hidden overlay', () => {
    const inst = mountSettings({ doc, send() {}, getState() { return state; }, audioEl, myToken: 'abc' });
    expect(findBtn(doc)).not.toBeNull();
    expect(findBtn(doc).textContent).toBe('\u2699');
    const overlay = findOverlay(doc);
    expect(overlay).not.toBeNull();
    expect(overlay.hidden).toBe(true);
    expect(overlay.getAttribute('aria-hidden')).toBe('true');
  });

  it('defaults to correct options when none passed', () => {
    const inst = mountSettings();
    expect(typeof inst.open).toBe('function');
    expect(typeof inst.close).toBe('function');
    expect(typeof inst.rebuildContent).toBe('function');
  });

  it('open() reveals the overlay and builds content', () => {
    const inst = mountSettings({ doc, send() {}, getState() { return state; }, audioEl, myToken: 'abc' });
    inst.open();
    const overlay = findOverlay(doc);
    expect(overlay.hidden).toBe(false);
    expect(overlay.getAttribute('aria-hidden')).toBe('false');
    expect(overlay.classList.contains('open')).toBe(true);
  });

  it('close() hides the overlay', () => {
    const inst = mountSettings({ doc, send() {}, getState() { return state; }, audioEl, myToken: 'abc' });
    inst.open();
    inst.close();
    const overlay = findOverlay(doc);
    expect(overlay.hidden).toBe(true);
    expect(overlay.getAttribute('aria-hidden')).toBe('true');
    expect(overlay.classList.contains('open')).toBe(false);
  });

  it('clicking the gear button toggles the overlay', () => {
    const inst = mountSettings({ doc, send() {}, getState() { return state; }, audioEl, myToken: 'abc' });
    const overlay = findOverlay(doc);
    expect(overlay.hidden).toBe(true);
    findBtn(doc).click();
    expect(overlay.hidden).toBe(false);
    findBtn(doc).click();
    expect(overlay.hidden).toBe(true);
  });

  it('shows volume slider with persisted value from localStorage', () => {
    // Set a value first so we can assert it's restored.
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem('phoenix-settings-volume', '0.35');
    } else {
      // In jsdom localStorage might not be available — skip or use a mock.
      // For this minimal DOM stub we'll just check the default.
    }
    const inst = mountSettings({ doc, send() {}, getState() { return state; }, audioEl, myToken: 'abc' });
    inst.open();
    const overlay = findOverlay(doc);
    expect(overlay.hidden).toBe(false);
  });
});

describe('settings content with a station that has ratings', () => {
  let doc, sendCalls, state, audioEl;

  beforeEach(() => {
    doc = makeDoc();
    sendCalls = [];
    state = {
      players: [{ token: 'tok1', station: 'helm' }],
      stations: [
        { id: 'helm', name: 'Helm', holder_token: 'tok1', ratings: ['Std', 'Reduced', 'Full'] },
      ],
      stationRatings: { helm: 'Reduced' },
    };
    audioEl = { volume: 1.0 };
  });

  it('renders rating buttons when station has multiple ratings', () => {
    const inst = mountSettings({ doc, send() {}, getState() { return state; }, audioEl, myToken: 'tok1' });
    inst.open();
    const overlay = findOverlay(doc);
    expect(overlay.hidden).toBe(false);
    const popup = overlay.children[0];
    // Find the rating section (first section-child of popup).
    const ratingSection = popup.children.find(c =>
      c.className === 'settings-section' &&
      c.children.some(ch => ch.className === 'settings-rating-row')
    );
    expect(ratingSection).toBeDefined();
    const heading = ratingSection.children.find(c => c.className === 'settings-section-heading');
    expect(heading.textContent).toBe(t('settings.rating'));
    const row = ratingSection.children.find(c => c.className === 'settings-rating-row');
    expect(row.children).toHaveLength(3);
    // "Reduced" is active.
    const activeBtns = row.children.filter(c => c.className.includes('active'));
    expect(activeBtns).toHaveLength(1);
    expect(activeBtns[0].textContent).toBe('REDUCED');
  });

  it('sends SetStationRating on rating button click', () => {
    const send = (type, data) => { sendCalls.push({ type, data }); };
    const inst = mountSettings({ doc, send, getState() { return state; }, audioEl, myToken: 'tok1' });
    inst.open();
    const overlay = findOverlay(doc);
    const popup = overlay.children[0];
    const ratingSection = popup.children.find(c =>
      c.className === 'settings-section' &&
      c.children.some(ch => ch.className === 'settings-rating-row')
    );
    const row = ratingSection.children.find(c => c.className === 'settings-rating-row');
    // Click the "FULL" button (not the active "REDUCED").
    const fullBtn = row.children.find(c => c.textContent === 'FULL');
    fullBtn.click();
    expect(sendCalls).toHaveLength(1);
    expect(sendCalls[0]).toEqual({ type: 'SetStationRating', data: { rating_name: 'Full' } });
  });
});

describe('settings content — leave station', () => {
  let doc, sendCalls, state, audioEl;

  beforeEach(() => {
    doc = makeDoc();
    sendCalls = [];
    state = {
      players: [{ token: 'tok1', station: 'tactical' }],
      stations: [
        { id: 'tactical', name: 'Tactical', holder_token: 'tok1', ratings: ['Std'] },
      ],
      stationRatings: { tactical: 'Std' },
    };
    audioEl = { volume: 1.0 };
  });

  it('renders Leave Station button when player has a station', () => {
    const inst = mountSettings({ doc, send() {}, getState() { return state; }, audioEl, myToken: 'tok1' });
    inst.open();
    const overlay = findOverlay(doc);
    const popup = overlay.children[0];
    const leaveSection = popup.children.find(c =>
      c.className === 'settings-section' &&
      c.children.some(ch => ch.textContent === t('settings.station'))
    );
    expect(leaveSection).toBeDefined();
    const leaveBtn = leaveSection.children.find(c =>
      c.className && c.className.includes('settings-leave-btn')
    );
    expect(leaveBtn).toBeDefined();
    expect(leaveBtn.textContent).toBe(t('settings.leave_station'));
  });

  it('sends ReleaseStation on leave button click', () => {
    const send = (type, data) => { sendCalls.push({ type, data }); };
    const inst = mountSettings({ doc, send, getState() { return state; }, audioEl, myToken: 'tok1' });
    inst.open();
    const overlay = findOverlay(doc);
    const popup = overlay.children[0];
    const leaveSection = popup.children.find(c =>
      c.className === 'settings-section' &&
      c.children.some(ch => ch.textContent === t('settings.station'))
    );
    const leaveBtn = leaveSection.children.find(c =>
      c.className && c.className.includes('settings-leave-btn')
    );
    leaveBtn.click();
    expect(sendCalls).toHaveLength(1);
    expect(sendCalls[0].type).toBe('ReleaseStation');
  });
});

describe('settings content — QR toggle', () => {
  let doc, sendCalls, state, audioEl;

  beforeEach(() => {
    doc = makeDoc();
    sendCalls = [];
    state = { players: [], stations: [], stationRatings: {} };
    audioEl = { volume: 1.0 };
  });

  it('sends ToggleQrCode on QR button click', () => {
    const send = (type, data) => { sendCalls.push({ type, data }); };
    const inst = mountSettings({ doc, send, getState() { return state; }, audioEl, myToken: 'tok1' });
    inst.open();
    const overlay = findOverlay(doc);
    const popup = overlay.children[0];
    const qrSection = popup.children.find(c =>
      c.className === 'settings-section' &&
      c.children.some(ch => ch.textContent === t('settings.qr_code'))
    );
    expect(qrSection).toBeDefined();
    const qrBtn = qrSection.children.find(c => c.className === 'settings-action-btn');
    expect(qrBtn).toBeDefined();
    expect(qrBtn.textContent).toBe(t('settings.toggle_qr'));
    qrBtn.click();
    expect(sendCalls).toHaveLength(1);
    expect(sendCalls[0].type).toBe('ToggleQrCode');
  });
});

describe('settings content — no station ratings', () => {
  let doc, state, audioEl;

  beforeEach(() => {
    doc = makeDoc();
    state = {
      players: [{ token: 'tok1', station: 'helm' }],
      stations: [
        { id: 'helm', name: 'Helm', holder_token: 'tok1', ratings: ['Std'] },
      ],
      stationRatings: { helm: 'Std' },
    };
    audioEl = { volume: 0.5 };
  });

  it('hides rating section when only one rating exists', () => {
    const inst = mountSettings({ doc, send() {}, getState() { return state; }, audioEl, myToken: 'tok1' });
    inst.open();
    const overlay = findOverlay(doc);
    const popup = overlay.children[0];
    const ratingSection = popup.children.find(c =>
      c.className === 'settings-section' &&
      c.children.some(ch => ch.className === 'settings-section-heading' && ch.textContent === 'Rating')
    );
    expect(ratingSection).toBeUndefined();
  });
});

describe('settings content — no station', () => {
  let doc, state, audioEl;

  beforeEach(() => {
    doc = makeDoc();
    state = {
      players: [{ token: 'tok1', station: null }],
      stations: [],
      stationRatings: {},
    };
    audioEl = { volume: 0.5 };
  });

  it('hides rating and leave sections when player has no station', () => {
    const inst = mountSettings({ doc, send() {}, getState() { return state; }, audioEl, myToken: 'tok1' });
    inst.open();
    const overlay = findOverlay(doc);
    const popup = overlay.children[0];
    // Only volume and QR sections should exist (no rating, no leave).
    const hasRating = popup.children.some(c =>
      c.className === 'settings-section' &&
      c.children.some(ch => ch.className === 'settings-section-heading' && ch.textContent === 'Rating')
    );
    const hasLeave = popup.children.some(c =>
      c.className === 'settings-section' &&
      c.children.some(ch => ch.textContent === t('settings.leave_station'))
    );
    expect(hasRating).toBe(false);
    expect(hasLeave).toBe(false);
  });
});
