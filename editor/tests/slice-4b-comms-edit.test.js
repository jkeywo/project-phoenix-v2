import { describe, it, expect, beforeEach } from 'vitest';
import { ModeShell } from '../mode-shell.js';

/**
 * Slice 4b integration test: comms editor.
 *
 * Exercises `renderCommsPanel` end-to-end against a fixture world that
 * mirrors `assets/worlds/default.toml`'s "Starbase Alpha on_hailed" comms
 * template. Verifies:
 *   - Editing the body textarea mutates layer.toml.comms[i].message.
 *   - Adding an action to a response appends to
 *     layer.toml.comms[i].response[j].action[].
 *   - Adding a follow-up creates
 *     layer.toml.comms[i].response[j].follow_up = { message, response }.
 *   - snapshotForUndo is called on every mutation (undo stack grows by 1).
 *   - Trigger <select> only exposes the three supported variants
 *     (on_attacked / on_destroyed / on_hailed).
 *
 * Uses the same FakeElement DOM shim style as slice-4a-trigger-edit.test.js.
 */

// ── Minimal DOM shim (reused from slice-4a-trigger-edit.test.js) ────────

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
    this.placeholder = '';
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
    const m = s.match(/^([a-z]+)#(.+)$/i);
    if (m) return el.tagName === m[1].toUpperCase() && el.id === m[2];
    return el.tagName === s.toUpperCase();
  };
}

class FakeDocument {
  createElement(tag) { return new FakeElement(tag); }
  getElementById() { return null; }
}

function installDom() {
  globalThis.document = new FakeDocument();
}

function fireInput(el, value) {
  el.value = value;
  el.dispatchEvent({ type: 'input', target: el });
}
function fireChange(el, value) {
  el.value = value;
  el.dispatchEvent({ type: 'change', target: el });
}
function fireClick(el) {
  el.dispatchEvent({ type: 'click', target: el });
}

// ── Fixture ─────────────────────────────────────────────────────────────

function makeWorld() {
  return {
    global: { seed: 42 },
    entity: [
      { template_path: 'assets/entities/starbase.toml', name: 'Starbase Alpha' },
    ],
    comms: [
      {
        from: 'Starbase Alpha',
        trigger: 'on_hailed',
        entity: 'Starbase Alpha',
        message: 'USS Phoenix, this is Starbase Alpha. Please state your business.',
        response: [
          {
            text: 'We are on a survey mission.',
            action: [
              {
                type: 'add_objective',
                id: 'obj-survey',
                text: 'Complete the survey in this sector.',
              },
            ],
          },
        ],
      },
    ],
  };
}

// ── Tests ────────────────────────────────────────────────────────────────

describe('Slice 4b: comms editor', () => {
  let modeShell;
  let layer;
  let host;
  let renderCommsPanel;

  beforeEach(async () => {
    installDom();
    modeShell = new ModeShell();
    globalThis.window = { __editorV2: { modeShell } };

    layer = {
      filename: 'assets/worlds/default.toml',
      toml: makeWorld(),
      isDirty: false,
    };
    host = new FakeElement('div');

    ({ renderCommsPanel } = await import('../comms-view.js'));
  });

  function getDeps() {
    return {
      allLayers: [{ path: layer.filename, worldState: layer.toml }],
      canvasManager: { renderAll: () => {} },
      layerManager: { getLayers: () => [layer], getActiveLayer: () => layer },
    };
  }

  it('renders the initial comms template body and response', () => {
    renderCommsPanel(host, { type: 'comms', commsIndex: 0, layer }, getDeps());

    const ta = host.querySelectorAll('textarea').find((t) => t.classList.contains('comms-body-input'));
    expect(ta).toBeDefined();
    expect(ta.value).toContain('Please state your business.');

    const responseCards = host.querySelectorAll('div').filter((d) => d.classList.contains('response-card'));
    expect(responseCards).toHaveLength(1);

    const respTextInput = responseCards[0]
      .querySelectorAll('input')
      .find((i) => i.classList.contains('response-text-input'));
    expect(respTextInput).toBeDefined();
    expect(respTextInput.value).toBe('We are on a survey mission.');
  });

  it('trigger select exposes only on_attacked / on_destroyed / on_hailed', () => {
    renderCommsPanel(host, { type: 'comms', commsIndex: 0, layer }, getDeps());

    const triggerSelect = host.querySelectorAll('select').find((s) => s.id === 'commsTriggerKind');
    expect(triggerSelect).toBeDefined();
    const values = triggerSelect.children.map((o) => o.value);
    // Three known + zero extras (current value 'on_hailed' is in the list).
    expect(values).toEqual(['on_attacked', 'on_destroyed', 'on_hailed']);
    expect(values).not.toContain('on_timer');
  });

  it('editing the body textarea writes back to layer.toml.comms[0].message + 1 undo snapshot', () => {
    renderCommsPanel(host, { type: 'comms', commsIndex: 0, layer }, getDeps());

    const ta = host.querySelectorAll('textarea').find((t) => t.classList.contains('comms-body-input'));
    fireInput(ta, 'New hail message.');

    expect(layer.toml.comms[0].message).toBe('New hail message.');
    expect(layer.isDirty).toBe(true);

    const undoEntries = modeShell.getUndoHistory('World', layer.filename);
    expect(undoEntries.length).toBe(1);
    // Snapshot is PRE-mutation → original message.
    expect(undoEntries[0].comms[0].message).toContain('Please state your business.');
  });

  it('+ Add Action on a response appends to layer.toml.comms[0].response[0].action', () => {
    renderCommsPanel(host, { type: 'comms', commsIndex: 0, layer }, getDeps());

    const card = host.querySelectorAll('div').find((d) => d.classList.contains('response-card'));
    const select = card.querySelectorAll('select').find((s) => s.classList.contains('newCommsActionType'));
    expect(select).toBeDefined();
    select.value = 'complete_objective';
    const addBtn = card.querySelectorAll('button').find((b) => b.classList.contains('btn-add-comms-action'));
    fireClick(addBtn);

    const actions = layer.toml.comms[0].response[0].action;
    expect(actions).toHaveLength(2);
    expect(actions[1].type).toBe('complete_objective');
    expect(layer.isDirty).toBe(true);

    const undoEntries = modeShell.getUndoHistory('World', layer.filename);
    expect(undoEntries.length).toBe(1);
  });

  it('+ Add Follow-Up Node creates comms[0].response[0].follow_up = { message, response? }', () => {
    renderCommsPanel(host, { type: 'comms', commsIndex: 0, layer }, getDeps());

    const card = host.querySelectorAll('div').find((d) => d.classList.contains('response-card'));
    const addBtn = card.querySelectorAll('button').find((b) => b.classList.contains('btn-add-follow-up'));
    expect(addBtn).toBeDefined();
    fireClick(addBtn);

    const follow = layer.toml.comms[0].response[0].follow_up;
    expect(follow).toBeDefined();
    expect(follow.message).toBe('');
    // The adapter strips empty responses arrays on the write-back path; OK.
    expect(follow.response === undefined || Array.isArray(follow.response)).toBe(true);
    expect(layer.isDirty).toBe(true);

    const undoEntries = modeShell.getUndoHistory('World', layer.filename);
    expect(undoEntries.length).toBe(1);
  });

  it('editing the from input mutates layer.toml.comms[0].from + 1 undo snapshot', () => {
    renderCommsPanel(host, { type: 'comms', commsIndex: 0, layer }, getDeps());

    const fromInput = host.querySelectorAll('input').find((i) => i.id === 'commsFrom');
    expect(fromInput).toBeDefined();
    fireInput(fromInput, 'Federation Command');

    expect(layer.toml.comms[0].from).toBe('Federation Command');
    expect(layer.isDirty).toBe(true);

    const undoEntries = modeShell.getUndoHistory('World', layer.filename);
    expect(undoEntries.length).toBe(1);
  });
});
