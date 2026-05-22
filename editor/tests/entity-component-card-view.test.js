import { describe, it, expect, beforeEach } from 'vitest';

// ── DOM shim (shared with other view tests) ──────────────────────────────

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
    this.style = {};
    this._innerHTML = '';
    this.textContent = '';
    this.value = '';
    this.checked = false;
    this.disabled = false;
    this.multiple = false;
    this.type = '';
    this.step = '';
    this.rows = 0;
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
      if (sel.startsWith('#')) return false;
      return el.tagName === sel.toUpperCase();
    });
  }
  querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
}

function installDom() {
  globalThis.document = { createElement: (tag) => new FakeElement(tag) };
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

// ── Tests ────────────────────────────────────────────────────────────────

describe('renderEntityComponentCard', () => {
  let host;
  let renderEntityComponentCard;
  let ComponentCard;
  let COMPONENT_SCHEMA;

  beforeEach(async () => {
    installDom();
    host = new FakeElement('div');
    ({ renderEntityComponentCard } = await import('../entity-component-card-view.js'));
    ({ ComponentCard } = await import('../entity-mode.js'));
    ({ COMPONENT_SCHEMA } = await import('../component-schema.js'));
  });

  function makeDeps(overrides = {}) {
    const edits = [];
    return {
      edits,
      deps: {
        getFactionOptions: () => [{ uuid: 'f-1', name: 'Federation' }],
        getComplexityPaths: () => ['assets/complexity/tactical.toml'],
        onEdit: (section, data) => edits.push({ section, data }),
        onDelete: (section) => edits.push({ section, deleted: true }),
        ...overrides,
      },
    };
  }

  it('renders a numeric input for hull.captain_chair that coerces to a number', () => {
    const card = new ComponentCard('hull', { captain_chair: 60 }, COMPONENT_SCHEMA.hull);
    const { deps, edits } = makeDeps();
    renderEntityComponentCard(host, card, deps);

    const inputs = host.querySelectorAll('input').filter((i) => i.type === 'number');
    const inp = inputs.find((i) => i.parentElement && i.parentElement.children[0]?.textContent === 'captain_chair');
    expect(inp).toBeDefined();
    expect(inp.value).toBe('60');

    fireInput(inp, '99.5');
    expect(edits.length).toBe(1);
    expect(edits[0].section).toBe('hull');
    expect(edits[0].data.captain_chair).toBe(99.5);
    expect(typeof edits[0].data.captain_chair).toBe('number');
  });

  it('renders a string input that coerces to a string', () => {
    const card = new ComponentCard('name', { name: 'Axiom' }, COMPONENT_SCHEMA.name);
    const { deps, edits } = makeDeps();
    renderEntityComponentCard(host, card, deps);

    const inputs = host.querySelectorAll('input').filter((i) => i.type === 'text');
    const nameInp = inputs.find((i) => i.parentElement?.children[0]?.textContent === 'name');
    expect(nameInp).toBeDefined();
    fireInput(nameInp, 'New Name');
    expect(edits[0].section).toBe('name');
    expect(edits[0].data.name).toBe('New Name');
  });

  it('renders a checkbox for boolean helm_console.radar_shows', () => {
    const card = new ComponentCard(
      'helm_console',
      { max_speed: 50, max_reverse_speed: 0, acceleration: 16, deceleration: 50, max_yaw_rate: 1.5, radar_range: 0, radar_shows: false, impulse_charge_duration: 3, impulse_speed_multiplier: 10 },
      COMPONENT_SCHEMA.helm_console,
    );
    const { deps, edits } = makeDeps();
    renderEntityComponentCard(host, card, deps);
    const cb = host.querySelectorAll('input').find((i) => i.type === 'checkbox');
    expect(cb).toBeDefined();
    cb.checked = true;
    cb.dispatchEvent({ type: 'change', target: cb });
    const last = edits[edits.length - 1];
    expect(last.data.radar_shows).toBe(true);
  });

  it('renders an array<number> textarea that coerces to numbers', () => {
    const card = new ComponentCard('radar_appearance', { colour: [0.6, 0.8, 1.0] }, COMPONENT_SCHEMA.radar_appearance);
    const { deps, edits } = makeDeps();
    renderEntityComponentCard(host, card, deps);

    const ta = host.querySelectorAll('textarea').find((t) => t.classList.contains('entity-card-input-array'));
    expect(ta).toBeDefined();
    expect(ta.value).toBe('0.6, 0.8, 1');
    fireInput(ta, '0.1, 0.2, 0.3');
    expect(edits[edits.length - 1].data.colour).toEqual([0.1, 0.2, 0.3]);
  });

  it('renders an array<string> textarea that coerces to strings', () => {
    const card = new ComponentCard('tags', ['ship', 'npc'], COMPONENT_SCHEMA.tags);
    // tags is an array section; the data IS the array. The schema treats it
    // as scalar-section behaviour. We test that.
    const { deps, edits } = makeDeps();
    renderEntityComponentCard(host, card, deps);
    const ta = host.querySelectorAll('textarea').find((t) => t.classList.contains('entity-card-input-array'));
    expect(ta).toBeDefined();
    fireInput(ta, 'ship, station, enemy');
    expect(edits[edits.length - 1].data).toEqual(['ship', 'station', 'enemy']);
  });

  it('raw-toggle replaces body with a textarea containing the section TOML', () => {
    const card = new ComponentCard('hull', { captain_chair: 60 }, COMPONENT_SCHEMA.hull);
    card.toggleRaw();
    const { deps, edits } = makeDeps();
    renderEntityComponentCard(host, card, deps);

    const ta = host.querySelectorAll('textarea').find((t) => t.classList.contains('entity-card-raw-textarea'));
    expect(ta).toBeDefined();
    expect(ta.value).toContain('hull');
    expect(ta.value).toContain('60');

    // Edit it: change captain_chair to 80.
    fireInput(ta, '[hull]\ncaptain_chair = 80\n');
    const last = edits[edits.length - 1];
    expect(last.section).toBe('hull');
    expect(last.data.captain_chair).toBe(80);
  });

  it('renders raw textarea when schema is null (null-schema fallback)', () => {
    const card = new ComponentCard('unknown_section', { foo: 1 }, null);
    const { deps } = makeDeps();
    renderEntityComponentCard(host, card, deps);

    const ta = host.querySelectorAll('textarea').find((t) => t.classList.contains('entity-card-raw-textarea'));
    expect(ta).toBeDefined();
    expect(ta.value).toContain('unknown_section');
  });

  it('faction field renders a <select> populated from getFactionOptions', () => {
    const card = new ComponentCard('faction', 'f-1', COMPONENT_SCHEMA.faction);
    const { deps, edits } = makeDeps();
    renderEntityComponentCard(host, card, deps);

    const sel = host.querySelectorAll('select').find((s) => s.classList.contains('entity-card-input-faction'));
    expect(sel).toBeDefined();
    // 1 blank + 1 federation option.
    expect(sel.children.length).toBe(2);
    expect(sel.value).toBe('f-1');

    fireChange(sel, '');
    expect(edits[edits.length - 1].data).toBeNull();
  });

  it('complexity_toml field renders a <select> populated from getComplexityPaths', () => {
    const card = new ComponentCard(
      'weapons_console',
      { complexity_toml: 'assets/complexity/tactical.toml', radar_range: 0, target_range: 0, fire_arc: 0, beam_range: 0, beam_damage_per_sec: 0, beam_duration_secs: 0, cooldown_secs: 0, beam_color: [] },
      COMPONENT_SCHEMA.weapons_console,
    );
    const { deps } = makeDeps();
    renderEntityComponentCard(host, card, deps);

    const sel = host.querySelectorAll('select').find((s) => s.classList.contains('entity-card-input-complexity'));
    expect(sel).toBeDefined();
    expect(sel.children.length).toBe(2);
    expect(sel.value).toBe('assets/complexity/tactical.toml');
  });

  it('header delete button calls onDelete', () => {
    const card = new ComponentCard('hull', { captain_chair: 60 }, COMPONENT_SCHEMA.hull);
    const { deps, edits } = makeDeps();
    renderEntityComponentCard(host, card, deps);

    const delBtn = host.querySelectorAll('button').find((b) => b.classList.contains('entity-card-btn-delete'));
    expect(delBtn).toBeDefined();
    fireClick(delBtn);
    expect(edits.find((e) => e.deleted)).toBeDefined();
  });

  it('collapse toggle hides body', () => {
    const card = new ComponentCard('hull', { captain_chair: 60 }, COMPONENT_SCHEMA.hull);
    const { deps } = makeDeps();
    renderEntityComponentCard(host, card, deps);

    const collapseBtn = host.querySelectorAll('button').find((b) => b.classList.contains('entity-card-btn-collapse'));
    fireClick(collapseBtn);
    const body = host.querySelectorAll('div').find((d) => d.classList.contains('entity-card-body'));
    expect(body.classList.contains('hidden')).toBe(true);
  });
});
