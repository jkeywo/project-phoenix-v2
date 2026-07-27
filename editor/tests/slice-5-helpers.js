/**
 * Shared scaffolding for Slice 5 Entity Mode integration tests.
 *
 *   import { setupEntityMode } from './slice-5-helpers.js';
 *   const { view, host, modeShell, saveFlow, restoreCb, writeFileCalls } =
 *     await setupEntityMode();
 *
 * The helper installs a minimal FakeElement DOM shim, a Konva stub, and
 * mounts `entity-mode-view` with mocked I/O that reads real fixtures from
 * `assets/entities/`.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// ── DOM shim ──────────────────────────────────────────────────────────────

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

export class FakeElement {
  constructor(tag) {
    this.tagName = (tag || 'div').toUpperCase();
    this.children = [];
    this.parentElement = null;
    this._listeners = new Map();
    this.classList = new FakeClassList();
    this.dataset = {};
    this.style = {};
    this._innerHTML = '';
    this.textContent = '';
    this.value = '';
    this.checked = false;
    this.disabled = false;
    this.multiple = false;
    this.type = '';
    this.id = '';
    this.placeholder = '';
    this.rows = 0;
    this.step = '';
    this._className = '';
  }
  get className() { return this._className; }
  set className(v) {
    this._className = v;
    this.classList = new FakeClassList();
    for (const c of String(v || '').split(/\s+/).filter(Boolean)) this.classList.add(c);
  }
  appendChild(c) {
    if (c.parentElement) {
      const i = c.parentElement.children.indexOf(c);
      if (i >= 0) c.parentElement.children.splice(i, 1);
    }
    c.parentElement = this;
    this.children.push(c);
    return c;
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
  _walk(pred, out = []) {
    if (pred(this)) out.push(this);
    for (const c of this.children) c._walk(pred, out);
    return out;
  }
  querySelectorAll(sel) {
    return this._walk((el) => {
      if (sel.startsWith('.')) return el.classList.contains(sel.slice(1));
      if (sel.startsWith('#')) return el.id === sel.slice(1);
      return el.tagName === sel.toUpperCase();
    });
  }
  querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
}

export function installDom() {
  globalThis.document = {
    createElement: (tag) => new FakeElement(tag),
    getElementById: () => null,
  };
}

export function fireInput(el, value) {
  el.value = value;
  el.dispatchEvent({ type: 'input', target: el });
}
export function fireChange(el, value) {
  el.value = value;
  el.dispatchEvent({ type: 'change', target: el });
}
export function fireClick(el) {
  el.dispatchEvent({ type: 'click', target: el });
}

// ── Konva stub ────────────────────────────────────────────────────────────

export const KonvaStub = (() => {
  class Stage { constructor() {} add() {} }
  class Layer { add() {} }
  const shape = function () {};
  return {
    Stage, Layer,
    Circle: shape, Rect: shape, Ring: shape, Line: shape,
    Arc: shape, RegularPolygon: shape,
  };
})();

// ── Real fixtures ─────────────────────────────────────────────────────────

const ROOT = resolve(import.meta.dirname, '..', '..');
export function fixture(rel) {
  return readFileSync(resolve(ROOT, rel), 'utf8');
}

// ── Mount helper ──────────────────────────────────────────────────────────

/**
 * @param {object} [opts]
 * @param {string[]} [opts.files]  assets/entities/*.toml filenames in the listing
 */
export async function setupEntityMode(opts = {}) {
  installDom();
  globalThis.window = { Konva: KonvaStub };
  const host = new FakeElement('div');

  const { mountEntityMode } = await import('../entity-mode-view.js');
  const { ModeShell } = await import('../mode-shell.js');
  const { SaveFlow } = await import('../save-flow.js');
  const { stringifyEntityToml } = await import('../entity-toml.js');

  const writeFileCalls = [];
  const writeFileFn = async (path, content) => {
    writeFileCalls.push({ path, content });
  };

  const modeShell = new ModeShell();
  modeShell.switchMode('Entity');
  const saveFlow = new SaveFlow(
    modeShell,
    { world: () => '', entity: stringifyEntityToml },
    writeFileFn,
    null,
  );

  let restoreCb = null;
  const registerRestore = (mode, fn) => {
    if (mode === 'Entity') restoreCb = fn;
  };

  const files = opts.files ?? ['ship_harrow_patrol.toml', 'alliance_battleship.toml'];

  const io = {
    readFile: async (path) => fixture(path),
    listDirectory: async (rel) => {
      if (rel === 'assets/entities') {
        return files.map((name) => ({ name, kind: 'file' }));
      }
      return [];
    },
    preload: async () => {},
    onCacheInvalidate: () => {},
    getProjectRoot: async () => ({ stub: true }),
    discover: async () => ({
      factionMap: new Map([
        ['aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa', 'Federation'],
        ['bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb', 'Pirate'],
      ]),
      complexityPaths: ['assets/complexity/tactical.toml'],
    }),
    Konva: KonvaStub,
  };

  const view = mountEntityMode({ host, modeShell, saveFlow, registerRestore, io });

  // Drain the fire-and-forget bootstrap.
  for (let i = 0; i < 10; i++) await Promise.resolve();
  await view._internal.refreshFileList();

  return {
    view,
    host,
    modeShell,
    saveFlow,
    writeFileCalls,
    getRestoreCb: () => restoreCb,
  };
}
