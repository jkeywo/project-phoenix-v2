import { describe, it, expect, beforeEach } from 'vitest';

// Reuse the minimal DOM shim.

class FakeClassList {
  constructor() { this._set = new Set(); }
  add(c) { this._set.add(c); }
  remove(c) { this._set.delete(c); }
  contains(c) { return this._set.has(c); }
}

class FakeElement {
  constructor(tag) {
    this.tagName = (tag || 'div').toUpperCase();
    this.children = [];
    this.parentElement = null;
    this._listeners = new Map();
    this.classList = new FakeClassList();
    this.dataset = {};
    this.textContent = '';
    this._innerHTML = '';
  }
  set className(v) {
    this.classList = new FakeClassList();
    for (const c of String(v || '').split(/\s+/).filter(Boolean)) this.classList.add(c);
    this._className = v;
  }
  get className() { return this._className || ''; }
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
      return el.tagName === sel.toUpperCase();
    });
  }
}

function installDom() {
  globalThis.document = { createElement: (tag) => new FakeElement(tag) };
}
function fireClick(el) { el.dispatchEvent({ type: 'click', target: el }); }

describe('renderAddComponentMenu', () => {
  let renderAddComponentMenu;
  let host;
  beforeEach(async () => {
    installDom();
    host = new FakeElement('div');
    ({ renderAddComponentMenu } = await import('../entity-add-component-menu.js'));
  });

  it('opens at the top level with combo entries and a "Raw section" toggle', () => {
    renderAddComponentMenu(host, () => {});
    const combos = host.querySelectorAll('button').filter((b) => b.classList.contains('entity-add-menu-combo'));
    expect(combos.length).toBeGreaterThanOrEqual(6); // Ship, Station, Region, NPC, Asteroid, Asteroid Field, Star, Planet
    const rawToggle = host.querySelectorAll('button').find((b) => b.classList.contains('entity-add-menu-raw-toggle'));
    expect(rawToggle).toBeDefined();
  });

  it('clicking the Raw section toggle shows the flat section submenu', () => {
    renderAddComponentMenu(host, () => {});
    const rawToggle = host.querySelectorAll('button').find((b) => b.classList.contains('entity-add-menu-raw-toggle'));
    fireClick(rawToggle);
    const submenu = host.querySelectorAll('div').find((d) => d.classList.contains('entity-add-menu-submenu'));
    expect(submenu).toBeDefined();
    const rawItems = host.querySelectorAll('button').filter((b) => b.classList.contains('entity-add-menu-raw-section'));
    // ENTITY_CONFIG_SECTIONS has many sections (tags through stations).
    expect(rawItems.length).toBeGreaterThan(10);
  });

  it('selecting a combo calls onSelect with { kind: "combo", name }', () => {
    let choice = null;
    renderAddComponentMenu(host, (c) => { choice = c; });
    const shipBtn = host.querySelectorAll('button').find((b) => b.dataset.combo === 'Ship');
    expect(shipBtn).toBeDefined();
    fireClick(shipBtn);
    expect(choice).toEqual({ kind: 'combo', name: 'Ship' });
  });

  it('selecting a raw section calls onSelect with { kind: "raw", sectionKey }', () => {
    let choice = null;
    renderAddComponentMenu(host, (c) => { choice = c; });
    const rawToggle = host.querySelectorAll('button').find((b) => b.classList.contains('entity-add-menu-raw-toggle'));
    fireClick(rawToggle);

    const hullBtn = host.querySelectorAll('button').find((b) => b.dataset.section === 'hull');
    expect(hullBtn).toBeDefined();
    fireClick(hullBtn);
    expect(choice).toEqual({ kind: 'raw', sectionKey: 'hull' });
  });

  it('Back button returns to the top level', () => {
    renderAddComponentMenu(host, () => {});
    const rawToggle = host.querySelectorAll('button').find((b) => b.classList.contains('entity-add-menu-raw-toggle'));
    fireClick(rawToggle);
    const back = host.querySelectorAll('button').find((b) => b.classList.contains('entity-add-menu-back'));
    fireClick(back);
    const combos = host.querySelectorAll('button').filter((b) => b.classList.contains('entity-add-menu-combo'));
    expect(combos.length).toBeGreaterThan(0);
  });
});
