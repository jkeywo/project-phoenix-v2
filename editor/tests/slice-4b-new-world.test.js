import { describe, it, expect, beforeEach } from 'vitest';
import { LayerManager } from '../layers.js';

/**
 * Slice 4b: new-world-dialog test.
 *
 * Verifies:
 *  - Click with a unique filename → writeFile(path, content) called once.
 *  - Collision filename → validation error, NO writeFile call.
 *  - On success, new layer appended + active.
 *  - prompt() returning null cancels cleanly.
 */

class FakeClassList {
  constructor() { this._set = new Set(); }
  add(c) { this._set.add(c); }
  remove() {}
  toggle() {}
  contains(c) { return this._set.has(c); }
}

class FakeElement {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.parentElement = null;
    this._listeners = new Map();
    this.dataset = {};
    this.style = {};
    this.classList = new FakeClassList();
    this.textContent = '';
    this.id = '';
    this.type = '';
    this.value = '';
    this.disabled = false;
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
  addEventListener(t, f) {
    if (!this._listeners.has(t)) this._listeners.set(t, []);
    this._listeners.get(t).push(f);
  }
  dispatchEvent(ev) {
    const list = this._listeners.get(ev.type) || [];
    for (const fn of list) fn(ev);
  }
}

class FakeDocument {
  constructor() { this._byId = new Map(); }
  createElement(tag) { return new FakeElement(tag); }
  getElementById(id) { return this._byId.get(id) || null; }
  register(id, el) { el.id = id; this._byId.set(id, el); }
}

function fireClick(el) {
  el.dispatchEvent({ type: 'click', target: el });
}

describe('Slice 4b: new-world-dialog', () => {
  let layerManager;
  let parent;
  let sibling;
  let writeCalls;
  let alerts;
  let onCreatedCount;
  let mountNewWorldButton;

  beforeEach(async () => {
    globalThis.document = new FakeDocument();
    parent = new FakeElement('div');
    sibling = new FakeElement('button');
    sibling.parentElement = parent;
    parent.children.push(sibling);
    document.register('addLayerBtn', sibling);

    layerManager = new LayerManager();
    writeCalls = [];
    alerts = [];
    onCreatedCount = 0;

    globalThis.window = {
      prompt: (msg, def) => 'fresh_world.toml',
      alert: (msg) => alerts.push(msg),
      tomlParse: (text) => ({ global: { seed: 42 } }),
    };

    ({ mountNewWorldButton } = await import('../new-world-dialog.js'));
  });

  function makeDeps(overrides = {}) {
    return {
      layerManager,
      writeFile: async (path, content) => { writeCalls.push({ path, content }); },
      tomlParse: (text) => ({ global: { seed: 42 } }),
      onCreated: () => { onCreatedCount++; },
      getExistingPaths: () => layerManager.getLayers().map((l) => l.filename),
      ...overrides,
    };
  }

  it('click with unique filename → writeFile called once + layer appended', async () => {
    const btn = mountNewWorldButton(makeDeps());
    expect(btn).toBeDefined();
    expect(btn.textContent).toBe('+ New World');

    fireClick(btn);

    // drain microtasks from the async handler
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    expect(writeCalls).toHaveLength(1);
    expect(writeCalls[0].path).toBe('assets/worlds/fresh_world.toml');
    expect(writeCalls[0].content).toContain('[global]');
    expect(alerts).toHaveLength(0);

    const added = layerManager.getLayers().find((l) => l.filename === 'assets/worlds/fresh_world.toml');
    expect(added).toBeDefined();
    expect(layerManager.getActiveLayer()).toBe(added);
    expect(added.toml.global.seed).toBe(42);
    expect(onCreatedCount).toBe(1);
  });

  it('collision filename → validation error, NO writeFile call', async () => {
    layerManager.addInMemoryLayer('assets/worlds/dupe.toml', { global: { seed: 1 } });
    window.prompt = () => 'dupe.toml';

    const btn = mountNewWorldButton(makeDeps());
    fireClick(btn);

    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    expect(writeCalls).toHaveLength(0);
    expect(alerts).toHaveLength(1);
    expect(alerts[0]).toMatch(/already exists/i);
    expect(onCreatedCount).toBe(0);
  });

  it('prompt cancelled (null) → no writeFile, no alert', async () => {
    window.prompt = () => null;
    const btn = mountNewWorldButton(makeDeps());
    fireClick(btn);

    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    expect(writeCalls).toHaveLength(0);
    expect(alerts).toHaveLength(0);
    expect(onCreatedCount).toBe(0);
  });

  it('writeFile rejection surfaces an alert and does not push a layer', async () => {
    const btn = mountNewWorldButton(makeDeps({
      writeFile: async () => { throw new Error('Disk full'); },
    }));

    fireClick(btn);
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    expect(alerts).toHaveLength(1);
    expect(alerts[0]).toMatch(/Disk full/);
    expect(layerManager.getLayers()).toHaveLength(0);
    expect(onCreatedCount).toBe(0);
  });
});
