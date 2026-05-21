import { describe, it, expect, beforeEach } from 'vitest';
import { LayerManager } from '../layers.js';

/**
 * Slice 4b: triggerable-worlds-panel test.
 *
 * Verifies:
 *  - Panel lists union of listDirectory(assets/worlds) and load_world
 *    references, deduped, excluding nothing (file-backed open layers get
 *    an "Open" badge instead of an Activate button).
 *  - Activate adds a session-only layer with `_sessionOnly: true`.
 *  - Deactivate removes the session-only layer; non-session layers are
 *    NOT given a Deactivate button.
 *  - buildV2Layers-equivalent (layerManager.getLayers + filename/toml)
 *    includes the new layer after activation.
 */

// Minimal DOM shim — same shape as the other Slice 4 tests.

class FakeClassList {
  constructor() { this._set = new Set(); }
  add(c) { this._set.add(c); }
  remove(c) { this._set.delete(c); }
  toggle(c, force) {
    const has = this._set.has(c);
    const want = force === undefined ? !has : !!force;
    if (want) this._set.add(c); else this._set.delete(c);
    return want;
  }
  contains(c) { return this._set.has(c); }
}

class FakeElement {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.parentElement = null;
    this._listeners = new Map();
    this.attributes = {};
    this.dataset = {};
    this.style = {};
    this.classList = new FakeClassList();
    this._innerHTML = '';
    this.textContent = '';
    this.value = '';
    this.disabled = false;
    this.id = '';
    this.type = '';
    this._className = '';
  }
  get className() { return this._className; }
  set className(v) {
    this._className = v;
    this.classList = new FakeClassList();
    for (const c of String(v || '').split(/\s+/).filter(Boolean)) {
      this.classList.add(c);
    }
  }
  appendChild(child) {
    if (child.parentElement) {
      const idx = child.parentElement.children.indexOf(child);
      if (idx !== -1) child.parentElement.children.splice(idx, 1);
    }
    child.parentElement = this;
    this.children.push(child);
    return child;
  }
  addEventListener(type, fn) {
    if (!this._listeners.has(type)) this._listeners.set(type, []);
    this._listeners.get(type).push(fn);
  }
  dispatchEvent(ev) {
    const list = this._listeners.get(ev.type) || [];
    for (const fn of list) fn(ev);
  }
  set innerHTML(v) { this._innerHTML = v; this.children = []; }
  get innerHTML() { return this._innerHTML; }
  _findAll(predicate, out = []) {
    if (predicate(this)) out.push(this);
    for (const c of this.children) c._findAll(predicate, out);
    return out;
  }
  querySelectorAll(sel) {
    const match = parseSelector(sel);
    return this._findAll(match);
  }
  querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
}

function parseSelector(sel) {
  return (el) => {
    let s = sel;
    if (s.startsWith('.')) return el.classList.contains(s.slice(1));
    if (s.startsWith('#')) return el.id === s.slice(1);
    return el.tagName === s.toUpperCase();
  };
}

class FakeDocument {
  constructor() { this._byId = new Map(); }
  createElement(tag) { return new FakeElement(tag); }
  getElementById(id) { return this._byId.get(id) || null; }
  register(id, el) { el.id = id; this._byId.set(id, el); }
}

function installDom() {
  globalThis.document = new FakeDocument();
}
function fireClick(el) {
  el.dispatchEvent({ type: 'click', target: el });
}

describe('Slice 4b: triggerable-worlds-panel', () => {
  let layerManager;
  let container;
  let renderTriggerableWorldsPanel;
  let onLayersChangedCount;

  beforeEach(async () => {
    installDom();
    globalThis.window = { alert: () => {} };

    layerManager = new LayerManager();
    container = new FakeElement('div');
    document.register('triggerableWorldsList', container);
    onLayersChangedCount = 0;

    ({ renderTriggerableWorldsPanel } = await import('../triggerable-worlds-panel.js'));
  });

  function getDeps(overrides = {}) {
    return {
      layerManager,
      onLayersChanged: () => { onLayersChangedCount++; },
      readFile: async () => '[global]\nseed = 7\n',
      listDirectory: async () => [
        { name: 'default.toml', kind: 'file' },
        { name: 'patrol.toml',  kind: 'file' },
        { name: 'README.md',    kind: 'file' },
      ],
      tomlParse: (text) => ({ global: { seed: 7 } }),
      ...overrides,
    };
  }

  it('lists candidates from listDirectory + referenced load_world paths', async () => {
    // Open a layer that references "assets/worlds/secret.toml" via load_world.
    layerManager.addInMemoryLayer('assets/worlds/default.toml', {
      global: { seed: 1 },
      trigger: [
        {
          condition: 'on_destroyed',
          entity: 'foo',
          action: [{ type: 'load_world', path: 'assets/worlds/secret.toml' }],
        },
      ],
    }, { sessionOnly: false });

    await renderTriggerableWorldsPanel(getDeps());

    const rows = container.querySelectorAll('div').filter((d) => d.classList.contains('triggerable-world-row'));
    const paths = rows.map((r) => r.dataset.path).sort();
    expect(paths).toEqual([
      'assets/worlds/default.toml',
      'assets/worlds/patrol.toml',
      'assets/worlds/secret.toml',
    ]);

    // The referenced-but-not-on-disk world has the (referenced) badge.
    const secretRow = rows.find((r) => r.dataset.path === 'assets/worlds/secret.toml');
    const referencedBadge = secretRow.children.find((c) => c.classList.contains('badge-referenced'));
    expect(referencedBadge).toBeDefined();
  });

  it('Activate button reads + parses the file and adds a session-only layer', async () => {
    await renderTriggerableWorldsPanel(getDeps());

    const rows = container.querySelectorAll('div').filter((d) => d.classList.contains('triggerable-world-row'));
    const patrolRow = rows.find((r) => r.dataset.path === 'assets/worlds/patrol.toml');
    expect(patrolRow).toBeDefined();
    const activateBtn = patrolRow.children.find((c) => c.tagName === 'BUTTON');
    expect(activateBtn.textContent).toBe('Activate');

    fireClick(activateBtn);

    // Activation is async (readFile + tomlParse). Drain microtasks.
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    const added = layerManager.getLayers().find((l) => l.filename === 'assets/worlds/patrol.toml');
    expect(added).toBeDefined();
    expect(added._sessionOnly).toBe(true);
    expect(added.toml.global.seed).toBe(7);
    expect(onLayersChangedCount).toBe(1);
  });

  it('Deactivate removes a session-only layer; file-backed layers stay put', async () => {
    // Pre-seed: one session-only, one file-backed.
    layerManager.addInMemoryLayer('assets/worlds/default.toml', { global: { seed: 1 } }, { sessionOnly: false });
    layerManager.addInMemoryLayer('assets/worlds/patrol.toml', { global: { seed: 2 } }, { sessionOnly: true });

    await renderTriggerableWorldsPanel(getDeps());

    const rows = container.querySelectorAll('div').filter((d) => d.classList.contains('triggerable-world-row'));

    // File-backed default.toml row: button says "Open" and is disabled.
    const defaultRow = rows.find((r) => r.dataset.path === 'assets/worlds/default.toml');
    const defaultBtn = defaultRow.children.find((c) => c.tagName === 'BUTTON');
    expect(defaultBtn.disabled).toBe(true);
    expect(defaultBtn.textContent).toBe('Open');

    // Session-only patrol row: button says "Deactivate".
    const patrolRow = rows.find((r) => r.dataset.path === 'assets/worlds/patrol.toml');
    const deactivateBtn = patrolRow.children.find((c) => c.tagName === 'BUTTON');
    expect(deactivateBtn.textContent).toBe('Deactivate');

    fireClick(deactivateBtn);

    expect(layerManager.getLayers().find((l) => l.filename === 'assets/worlds/patrol.toml')).toBeUndefined();
    // default.toml (file-backed) remains.
    expect(layerManager.getLayers().find((l) => l.filename === 'assets/worlds/default.toml')).toBeDefined();
    expect(onLayersChangedCount).toBe(1);
  });

  it('after activation, layerManager.getLayers (V2 view) includes the new layer', async () => {
    await renderTriggerableWorldsPanel(getDeps());

    const rows = container.querySelectorAll('div').filter((d) => d.classList.contains('triggerable-world-row'));
    const activateBtn = rows
      .find((r) => r.dataset.path === 'assets/worlds/default.toml')
      .children.find((c) => c.tagName === 'BUTTON');
    fireClick(activateBtn);
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    const v2Layers = layerManager.getLayers().map((l) => ({ path: l.filename, worldState: l.toml }));
    expect(v2Layers.find((l) => l.path === 'assets/worlds/default.toml')).toBeDefined();
  });

  it('shows placeholder when no root picked AND no layers open', async () => {
    await renderTriggerableWorldsPanel(getDeps({
      listDirectory: async () => { throw new Error('no root'); },
    }));

    const placeholder = container.querySelectorAll('p').find((p) => p.classList.contains('placeholder'));
    expect(placeholder).toBeDefined();
    expect(placeholder.textContent).toMatch(/pick project root/i);
  });
});
