import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach } from 'vitest';
import { JSDOM } from 'jsdom';
import {
  reconcileRows,
  keyedRebuild,
  setBtn,
  setBar,
  setAutoState,
  setText,
} from '../../gui/console-ui.js';

// ── Helpers ──────────────────────────────────────────────────────────────────

function makeDoc() {
  const dom = new JSDOM('<!DOCTYPE html><body></body>');
  return dom.window.document;
}

function makeDiv(doc) {
  return doc.createElement('div');
}

function makeBtn(doc) {
  const btn = doc.createElement('button');
  btn.className = 'btn';
  return btn;
}

// ── reconcileRows ─────────────────────────────────────────────────────────────

describe('reconcileRows', () => {
  let doc, container, cache;

  beforeEach(() => {
    doc = makeDoc();
    container = makeDiv(doc);
    doc.body.appendChild(container);
    cache = {};
  });

  function buildFn(id) {
    const row = doc.createElement('div');
    row.className = 'row';
    row.dataset.id = id;
    const label = doc.createElement('span');
    row.appendChild(label);
    return { row, label };
  }

  function updateFn(id, els, datum) {
    els.label.textContent = datum !== undefined ? String(datum) : id;
  }

  it('builds rows on first call', () => {
    reconcileRows(container, ['a', 'b'], cache, buildFn, updateFn, ['A', 'B']);
    expect(container.children.length).toBe(2);
    expect(container.children[0].dataset.id).toBe('a');
    expect(container.children[1].dataset.id).toBe('b');
    expect(cache['a']).toBeDefined();
    expect(cache['b']).toBeDefined();
  });

  it('updates existing rows on second call without rebuilding', () => {
    reconcileRows(container, ['a', 'b'], cache, buildFn, updateFn, ['First-A', 'First-B']);
    const rowARef = cache['a'].row;
    reconcileRows(container, ['a', 'b'], cache, buildFn, updateFn, ['Second-A', 'Second-B']);
    // Same node — not rebuilt
    expect(cache['a'].row).toBe(rowARef);
    expect(cache['a'].label.textContent).toBe('Second-A');
    expect(cache['b'].label.textContent).toBe('Second-B');
  });

  it('removes stale rows when ids are removed', () => {
    reconcileRows(container, ['a', 'b', 'c'], cache, buildFn, updateFn);
    expect(container.children.length).toBe(3);
    reconcileRows(container, ['a', 'c'], cache, buildFn, updateFn);
    expect(container.children.length).toBe(2);
    expect(cache['b']).toBeUndefined();
    expect(cache['a']).toBeDefined();
    expect(cache['c']).toBeDefined();
  });

  it('preserves order when ids are reordered', () => {
    reconcileRows(container, ['a', 'b', 'c'], cache, buildFn, updateFn);
    reconcileRows(container, ['c', 'a', 'b'], cache, buildFn, updateFn);
    expect(container.children[0].dataset.id).toBe('c');
    expect(container.children[1].dataset.id).toBe('a');
    expect(container.children[2].dataset.id).toBe('b');
  });

  it('calls updateFn for every row in order', () => {
    const calls = [];
    reconcileRows(
      container, ['x', 'y'], cache,
      buildFn,
      (id, els, datum) => { calls.push(id); },
      ['dx', 'dy']
    );
    expect(calls).toEqual(['x', 'y']);
  });
});

// ── keyedRebuild ──────────────────────────────────────────────────────────────

describe('keyedRebuild', () => {
  let doc, container;

  beforeEach(() => {
    doc = makeDoc();
    container = makeDiv(doc);
  });

  it('calls buildFn on first call (no key)', () => {
    let calls = 0;
    keyedRebuild(container, 'key1', () => { calls++; });
    expect(calls).toBe(1);
  });

  it('skips buildFn when key is unchanged', () => {
    let calls = 0;
    const fn = () => { calls++; };
    keyedRebuild(container, 'key1', fn);
    keyedRebuild(container, 'key1', fn);
    expect(calls).toBe(1);
  });

  it('rebuilds when key changes', () => {
    let calls = 0;
    const fn = () => { calls++; };
    keyedRebuild(container, 'key1', fn);
    keyedRebuild(container, 'key2', fn);
    expect(calls).toBe(2);
  });

  it('clears innerHTML on rebuild', () => {
    const doc2 = makeDoc();
    const el = doc2.createElement('div');
    keyedRebuild(el, 'k1', (c) => { c.innerHTML = '<span>hello</span>'; });
    expect(el.children.length).toBe(1);
    keyedRebuild(el, 'k2', (c) => { /* leave empty */ });
    expect(el.children.length).toBe(0);
  });

  it('converts numeric keys to string for comparison', () => {
    let calls = 0;
    const fn = () => { calls++; };
    keyedRebuild(container, 42, fn);
    keyedRebuild(container, 42, fn);
    expect(calls).toBe(1);
    keyedRebuild(container, 43, fn);
    expect(calls).toBe(2);
  });
});

// ── setBtn ────────────────────────────────────────────────────────────────────

describe('setBtn', () => {
  let doc, btn;

  beforeEach(() => {
    doc = makeDoc();
    btn = makeBtn(doc);
  });

  it('sets disabled=false when enabled=true (default)', () => {
    setBtn(btn, { enabled: true });
    expect(btn.disabled).toBe(false);
  });

  it('sets disabled=true when enabled=false', () => {
    setBtn(btn, { enabled: false });
    expect(btn.disabled).toBe(true);
  });

  it('adds disabled class when not enabled', () => {
    setBtn(btn, { enabled: false });
    expect(btn.className).toContain('disabled');
  });

  it('does not add disabled class when enabled', () => {
    setBtn(btn, { enabled: true });
    expect(btn.className).not.toContain('disabled');
  });

  it('adds variant class when enabled', () => {
    setBtn(btn, { enabled: true, variant: 'danger' });
    expect(btn.className).toContain('danger');
  });

  it('does not add variant class when disabled', () => {
    setBtn(btn, { enabled: false, variant: 'danger' });
    expect(btn.className).not.toContain('danger');
  });

  it('adds active class when active=true', () => {
    setBtn(btn, { active: true });
    expect(btn.className).toContain('active');
  });

  it('does not add active class when active=false', () => {
    setBtn(btn, { active: false });
    expect(btn.className).not.toContain('active');
  });

  it('uses custom base class', () => {
    setBtn(btn, { base: 'my-btn' });
    expect(btn.className).toContain('my-btn');
  });
});

// ── setBar ────────────────────────────────────────────────────────────────────

describe('setBar', () => {
  let doc, fillEl;

  beforeEach(() => {
    doc = makeDoc();
    fillEl = doc.createElement('div');
    fillEl.className = 'fill';
  });

  it('sets width as percentage string', () => {
    setBar(fillEl, 0.5);
    // jsdom normalises '50.0%' to '50%' — check the numeric value
    expect(parseFloat(fillEl.style.width)).toBeCloseTo(50, 1);
  });

  it('sets crit class at fraction <= 0.33 (default threshold)', () => {
    setBar(fillEl, 0.2);
    expect(fillEl.className).toContain('crit');
  });

  it('sets crit at exactly 0.33', () => {
    setBar(fillEl, 0.33);
    expect(fillEl.className).toContain('crit');
  });

  it('sets warn class at fraction <= 0.60 and > 0.33', () => {
    setBar(fillEl, 0.5);
    expect(fillEl.className).toContain('warn');
    expect(fillEl.className).not.toContain('crit');
  });

  it('sets no threshold class above 0.60', () => {
    setBar(fillEl, 0.8);
    expect(fillEl.className).not.toContain('warn');
    expect(fillEl.className).not.toContain('crit');
    expect(fillEl.className).toBe('fill');
  });

  it('clamps fraction below 0 to 0', () => {
    setBar(fillEl, -0.5);
    expect(parseFloat(fillEl.style.width || '0')).toBeCloseTo(0, 1);
  });

  it('clamps fraction above 1 to 1', () => {
    setBar(fillEl, 1.5);
    expect(parseFloat(fillEl.style.width)).toBeCloseTo(100, 1);
  });

  it('honours custom thresholds', () => {
    setBar(fillEl, 0.4, { thresholds: [[0.5, 'low']], baseClass: 'bar' });
    expect(fillEl.className).toContain('low');
  });

  it('uses custom baseClass', () => {
    setBar(fillEl, 0.9, { baseClass: 'progress' });
    expect(fillEl.className).toBe('progress');
  });
});

// ── setAutoState ──────────────────────────────────────────────────────────────

describe('setAutoState', () => {
  let doc, btn, badge;

  beforeEach(() => {
    doc = makeDoc();
    btn = makeBtn(doc);
    badge = doc.createElement('span');
    badge.hidden = true;
  });

  it('hides badge when isAuto=false', () => {
    setAutoState(btn, badge, false);
    expect(badge.hidden).toBe(true);
  });

  it('shows badge when isAuto=true', () => {
    setAutoState(btn, badge, true);
    expect(badge.hidden).toBe(false);
  });

  it('sets badge textContent to AUTO', () => {
    setAutoState(btn, badge, true);
    expect(badge.textContent).toBe(t('console.common.auto'));
  });

  it('disables button when isAuto=true', () => {
    setAutoState(btn, badge, true);
    expect(btn.disabled).toBe(true);
  });

  it('enables button when isAuto=false', () => {
    btn.disabled = true;
    setAutoState(btn, badge, false);
    expect(btn.disabled).toBe(false);
  });

  it('adds readonly class to button when isAuto=true', () => {
    setAutoState(btn, badge, true);
    expect(btn.classList.contains('readonly')).toBe(true);
  });

  it('removes readonly class from button when isAuto=false', () => {
    btn.classList.add('readonly');
    setAutoState(btn, badge, false);
    expect(btn.classList.contains('readonly')).toBe(false);
  });

  it('handles null button gracefully', () => {
    expect(() => setAutoState(null, badge, true)).not.toThrow();
    expect(badge.hidden).toBe(false);
  });

  it('handles null badge gracefully', () => {
    expect(() => setAutoState(btn, null, true)).not.toThrow();
    expect(btn.disabled).toBe(true);
  });

  it('handles both null button and badge gracefully', () => {
    expect(() => setAutoState(null, null, true)).not.toThrow();
  });
});

// ── setText ───────────────────────────────────────────────────────────────────

describe('setText', () => {
  let doc, el;

  beforeEach(() => {
    doc = makeDoc();
    el = doc.createElement('span');
    el.textContent = 'old';
    el.className = 'old-class';
  });

  it('sets textContent', () => {
    setText(el, 'hello');
    expect(el.textContent).toBe('hello');
  });

  it('converts non-string values to string', () => {
    setText(el, 42);
    expect(el.textContent).toBe('42');
  });

  it('sets className when cls is provided', () => {
    setText(el, 'x', { cls: 'new-class' });
    expect(el.className).toBe('new-class');
  });

  it('does not change className when cls is not provided', () => {
    setText(el, 'x');
    expect(el.className).toBe('old-class');
  });

  it('sets style.color when color is provided', () => {
    setText(el, 'x', { color: 'red' });
    expect(el.style.color).toBe('red');
  });

  it('does not change color when color is not provided', () => {
    el.style.color = 'blue';
    setText(el, 'x');
    expect(el.style.color).toBe('blue');
  });

  it('handles null element gracefully', () => {
    expect(() => setText(null, 'hello')).not.toThrow();
  });
});
