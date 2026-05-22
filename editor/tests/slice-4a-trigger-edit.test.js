import { describe, it, expect, beforeEach } from 'vitest';
import { ModeShell } from '../mode-shell.js';
import { createUndoController } from '../undo-controller.js';

/**
 * Slice 4a integration test: trigger editor.
 *
 * Exercises `renderTriggerPanel` end-to-end (build, mutate, add-action)
 * against a fixture world that mirrors `assets/worlds/default.toml`'s
 * raider-alpha trigger. Verifies the snapshot-before-mutate contract,
 * the dirty flag, and the schema-driven picker integrations (entity,
 * objective-id, file-picker).
 *
 * Vitest runs in node mode without jsdom, so we install a minimal DOM
 * shim — just enough for createElement / appendChild / event dispatch /
 * value & checked properties. Lives in this file to avoid a new dep.
 */

// ── Minimal DOM shim ────────────────────────────────────────────────────

class FakeClassList {
  constructor() { this._set = new Set(); }
  add(c)    { this._set.add(c); }
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
    this.checked = false;
    this.type = '';
    this.step = '';
    this.rows = 0;
    this.id = '';
    this._className = '';
  }
  get className() { return this._className; }
  set className(v) {
    this._className = v;
    // Mirror to classList so .contains() and querySelectorAll('.cls') work.
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
  replaceWith(other) {
    if (!this.parentElement) return;
    const idx = this.parentElement.children.indexOf(this);
    if (idx === -1) return;
    other.parentElement = this.parentElement;
    this.parentElement.children[idx] = other;
    this.parentElement = null;
  }
  addEventListener(type, fn) {
    if (!this._listeners.has(type)) this._listeners.set(type, []);
    this._listeners.get(type).push(fn);
  }
  dispatchEvent(ev) {
    const list = this._listeners.get(ev.type) || [];
    for (const fn of list) fn(ev);
  }
  set innerHTML(v) {
    this._innerHTML = v;
    this.children = [];
  }
  get innerHTML() { return this._innerHTML; }
  // Helpers for tests.
  _findAll(predicate, out = []) {
    if (predicate(this)) out.push(this);
    for (const c of this.children) c._findAll(predicate, out);
    return out;
  }
  querySelectorAll(sel) {
    // Supports `tag`, `#id`, `tag#id`, and `.class`.
    const match = parseSelector(sel);
    return this._findAll(match);
  }
  querySelector(sel) {
    return this.querySelectorAll(sel)[0] || null;
  }
}

function parseSelector(sel) {
  return (el) => {
    let s = sel;
    // .class
    if (s.startsWith('.')) return el.classList.contains(s.slice(1));
    // #id
    if (s.startsWith('#')) return el.id === s.slice(1);
    // tag#id
    const m = s.match(/^([a-z]+)#(.+)$/i);
    if (m) return el.tagName === m[1].toUpperCase() && el.id === m[2];
    // tag
    return el.tagName === s.toUpperCase();
  };
}

class FakeDocument {
  createElement(tag) { return new FakeElement(tag); }
  getElementById(_id) { return null; }
}

function installDom() {
  globalThis.document = new FakeDocument();
}

function fireChange(el, value) {
  el.value = value;
  el.dispatchEvent({ type: 'change', target: el });
}
function fireInput(el, value) {
  el.value = value;
  el.dispatchEvent({ type: 'input', target: el });
}
function fireClick(el) {
  el.dispatchEvent({ type: 'click', target: el });
}

// ── Fixture ─────────────────────────────────────────────────────────────

function makeWorld() {
  return {
    global: { seed: 42 },
    anchors: { patrol_alpha: [300.0, 0.0, -300.0] },
    entity: [
      {
        template_path: 'assets/entities/pirate_raider.toml',
        name: 'raider_alpha',
        transform: { anchor: 'patrol_alpha' },
      },
    ],
    trigger: [
      {
        condition: 'on_destroyed',
        entity: 'raider_alpha',
        action: [
          {
            type: 'add_objective',
            id: 'obj-raider-destroyed',
            text: 'Destroy the raider patrol',
            mandatory: true,
          },
        ],
      },
    ],
  };
}

// ── Tests ────────────────────────────────────────────────────────────────

describe('Slice 4a: trigger editor + action-card', () => {
  let modeShell;
  let undoController;
  let layer;
  let host;
  let renderTriggerPanel;

  beforeEach(async () => {
    installDom();
    modeShell = new ModeShell();
    undoController = createUndoController({ modeShell });

    layer = {
      filename: 'assets/worlds/default.toml',
      toml: makeWorld(),
      isDirty: false,
    };
    host = new FakeElement('div');

    // Import after globals are in place (module-level code is minimal but
    // safer to re-evaluate fresh each test). Vitest caches; we tolerate that.
    ({ renderTriggerPanel } = await import('../trigger-view.js'));
  });

  function getDeps(overrides = {}) {
    return {
      allLayers: [{ path: layer.filename, worldState: layer.toml }],
      canvasManager: { renderAll: () => {} },
      layerManager: { getLayers: () => [layer] },
      undoController,
      ...overrides,
    };
  }

  it('renders the existing add_objective action for raider_alpha', () => {
    renderTriggerPanel(host, { type: 'trigger', triggerIndex: 0, layer }, getDeps());

    const titles = host.querySelectorAll('span')
      .filter((e) => e.classList.contains('action-card-title'))
      .map((e) => e.textContent);
    expect(titles).toContain('▾ Add Objective');

    // The action list contains one card.
    const cards = host.querySelectorAll('div').filter((e) => e.classList.contains('action-card'));
    expect(cards).toHaveLength(1);
  });

  it('editing the action text writes back via onChange and pushes one undo snapshot', () => {
    renderTriggerPanel(host, { type: 'trigger', triggerIndex: 0, layer }, getDeps());

    // Find the input whose preceding sibling is the label "text".
    const fieldRows = host.querySelectorAll('div').filter((e) => e.classList.contains('action-field'));
    let textInput = null;
    for (const row of fieldRows) {
      const label = row.children.find((c) => c.tagName === 'LABEL');
      if (label && label.textContent === 'text') {
        textInput = row.children.find((c) => c.tagName === 'INPUT');
        break;
      }
    }
    expect(textInput).not.toBeNull();
    expect(textInput.value).toBe('Destroy the raider patrol');

    fireInput(textInput, 'Destroy the patrol now');

    expect(layer.toml.trigger[0].action[0].text).toBe('Destroy the patrol now');
    expect(layer.toml.trigger[0].action[0].id).toBe('obj-raider-destroyed');
    expect(layer.isDirty).toBe(true);

    const undoEntries = modeShell.getUndoHistory('World', layer.filename);
    expect(undoEntries.length).toBe(1);
    // The snapshot is the PRE-mutation state — the original text.
    expect(undoEntries[0].trigger[0].action[0].text).toBe('Destroy the raider patrol');
  });

  it('+ Add Action with complete_objective surfaces the same-world objective id picker', () => {
    renderTriggerPanel(host, { type: 'trigger', triggerIndex: 0, layer }, getDeps());

    const select = host.querySelectorAll('select').find((s) => s.id === 'newActionType');
    expect(select).toBeDefined();
    select.value = 'complete_objective';
    const addBtn = host.querySelectorAll('button').find((b) => b.id === 'addActionBtn');
    fireClick(addBtn);

    // World now has two actions.
    expect(layer.toml.trigger[0].action).toHaveLength(2);
    const newAction = layer.toml.trigger[0].action[1];
    expect(newAction.type).toBe('complete_objective');
    expect(newAction.id).toBe(''); // default for required string

    // After re-render, the second card's `id` field is a <select>
    // populated from add_objective in the same world.
    const titles = host.querySelectorAll('span')
      .filter((e) => e.classList.contains('action-card-title'))
      .map((e) => e.textContent);
    expect(titles).toEqual(['▾ Add Objective', '▾ Complete Objective']);

    // Find the 'id' select inside the second card.
    const cards = host.querySelectorAll('div').filter((e) => e.classList.contains('action-card'));
    const completeCard = cards[1];
    const idRow = completeCard.querySelectorAll('div')
      .filter((e) => e.classList.contains('action-field'))
      .find((row) => row.children.find((c) => c.tagName === 'LABEL' && c.textContent === 'id'));
    expect(idRow).toBeDefined();

    const idSelect = idRow.children.find((c) => c.tagName === 'SELECT');
    expect(idSelect).toBeDefined();
    const optionValues = idSelect.children.map((o) => o.value);
    expect(optionValues).toContain('obj-raider-destroyed');
  });

  it('+ Add Action with load_world renders a Pick… button using openFilePicker', async () => {
    let pickerCalls = 0;
    const deps = getDeps({
      openFilePicker: async (root) => {
        pickerCalls++;
        expect(root).toBe('assets/worlds/');
        return 'assets/worlds/patrol.toml';
      },
    });

    renderTriggerPanel(host, { type: 'trigger', triggerIndex: 0, layer }, deps);

    const select = host.querySelectorAll('select').find((s) => s.id === 'newActionType');
    select.value = 'load_world';
    const addBtn = host.querySelectorAll('button').find((b) => b.id === 'addActionBtn');
    fireClick(addBtn);

    // After re-render, second card should have a Pick… button.
    const cards = host.querySelectorAll('div').filter((e) => e.classList.contains('action-card'));
    const loadCard = cards[1];
    const pickBtn = loadCard.querySelectorAll('button').find((b) => b.textContent === 'Pick…');
    expect(pickBtn).toBeDefined();

    // Click it; this returns a promise via the async handler.
    fireClick(pickBtn);

    // Await microtasks to allow async picker to resolve.
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    expect(pickerCalls).toBe(1);
    expect(layer.toml.trigger[0].action[1].path).toBe('assets/worlds/patrol.toml');
  });
});
