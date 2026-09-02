// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  createGmActivityFeed,
  filterGmActivityEntries,
  parseGmActivityFeed,
  reduceGmActivityFeed,
} from '../../gui/gm-activity-feed.js';
import { buildTable, setTable, wireText } from '../../gui/strings.js';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '../..');
const read = (file) => fs.readFileSync(path.join(root, file), 'utf8');
const realStrings = buildTable(read('assets/strings/strings.csv'));

const SHIP = '00000000-0000-4000-8000-000000000001';
const SOURCE = '00000000-0000-4000-8000-000000000002';
const OTHER_SHIP = '00000000-0000-4000-8000-000000000003';

function reference(entity_id, name = entity_id) {
  return { entity_id, name };
}

const ship = reference(SHIP, 'entity.alliance_cruiser.display_name');
const source = reference(SOURCE, 'Raider');

function entry(category, detail, overrides = {}) {
  return {
    tick: 7,
    category,
    ships: [ship],
    links: [{ role: 'ship', entity: ship }],
    detail,
    ...overrides,
  };
}

function damage(overrides = {}) {
  return entry('damage', {
    type: 'damage',
    data: {
      victim_kind: 'ship',
      weapon: 'phaser-bank-1',
      amount: 4,
      shield_absorbed: 1,
      hull_damage: 3,
      system_hit: null,
    },
  }, {
    links: [{ role: 'source', entity: source }, { role: 'victim', entity: ship }],
    ...overrides,
  });
}

function destruction(overrides = {}) {
  return entry('destruction', { type: 'destruction' }, {
    links: [{ role: 'victim', entity: ship }],
    ...overrides,
  });
}

function objective(overrides = {}) {
  return entry('objective', {
    type: 'objective', data: { objective_id: 'reach_beacon', status: 'completed' },
  }, overrides);
}

function trigger(overrides = {}) {
  return entry('trigger', {
    type: 'trigger', data: { trigger_id: 'arrival', origin: 'worlds/test.rhai' },
  }, overrides);
}

function redAlert(overrides = {}) {
  return entry('red_alert', {
    type: 'red_alert', data: { active: true },
  }, overrides);
}

function connection(overrides = {}) {
  return entry('connection', {
    type: 'connection',
    data: {
      identity: { id: 'crew-1', name: 'Ari' },
      role: 'crew',
      state: 'connected',
      ship,
    },
  }, overrides);
}

function gmAction(outcome = 'applied', overrides = {}) {
  return entry('gm_action', {
    type: 'gm_action',
    data: {
      operator: { id: 'gm-alpha', name: 'Morgan' },
      correlation: `pause-${outcome}`,
      action: { type: 'set_session_paused', active: true },
      outcome,
      reason: outcome === 'refused' ? 'wrong-phase' : null,
      order: { sequence: 4, origin: 1 },
    },
  }, { ships: [], links: [], ...overrides });
}

function allCategories() {
  return [
    damage(),
    destruction(),
    objective(),
    trigger(),
    redAlert(),
    connection(),
    gmAction(),
  ];
}

function payload(entries, capacity = 128) {
  return { capacity, entries };
}

function mount({
  available = new Set([SHIP, SOURCE, OTHER_SHIP]),
  selectEntity = vi.fn(() => true),
} = {}) {
  document.body.innerHTML = `
    <section id="gm-activity"><h2 id="gm-activity-heading"></h2>
      <select id="gm-activity-category-filter">
        <option value="all">all</option>
        ${[...new Set(allCategories().map((candidate) => candidate.category))]
    .map((category) => `<option value="${category}">${category}</option>`).join('')}
      </select>
      <select id="gm-activity-ship-filter"></select>
      <button id="gm-activity-clear-filters"></button>
      <p id="gm-activity-status"></p>
      <p id="gm-activity-empty"></p>
      <ol id="gm-activity-list"></ol>
    </section>`;
  const feed = createGmActivityFeed({
    doc: document,
    t: (id, params = {}) => `${id}:${Object.values(params).join('/')}`,
    displayText: wireText,
    containsEntity: (id) => available.has(id),
    selectEntity,
  });
  return { feed, available, selectEntity };
}

describe('GM activity feed pure adapter', () => {
  it('accepts every detail under the common contract and strips unknown fields', () => {
    const raw = allCategories();
    raw[0] = {
      ...raw[0],
      private_component: 'Hull',
      detail: { ...raw[0].detail, data: { ...raw[0].detail.data, hidden: 99 } },
    };
    const parsed = parseGmActivityFeed(JSON.stringify(payload(raw, 16)));
    expect(parsed.entries.map(({ category }) => category)).toEqual([
      'damage', 'destruction', 'objective', 'trigger', 'red_alert', 'connection', 'gm_action',
    ]);
    for (const candidate of parsed.entries) {
      expect(Object.keys(candidate)).toEqual(['tick', 'category', 'ships', 'links', 'detail']);
      expect(candidate.detail.type).toBe(candidate.category);
    }
    expect(parsed.entries[0]).not.toHaveProperty('private_component');
    expect(parsed.entries[0].detail.data).not.toHaveProperty('hidden');
  });

  it('rejects category/detail mismatches and malformed detail variants', () => {
    expect(parseGmActivityFeed(payload([{ ...damage(), category: 'destruction' }])))
      .toBeUndefined();
    expect(parseGmActivityFeed(payload([{ ...objective(), detail: {
      type: 'objective', data: { objective_id: 'x', status: 'unknown' },
    } }]))).toBeUndefined();
    expect(parseGmActivityFeed(payload([{ ...gmAction(), detail: {
      ...gmAction().detail,
      data: { ...gmAction().detail.data, outcome: 'pending' },
    } }]))).toBeUndefined();
    expect(parseGmActivityFeed('{')).toBeUndefined();
  });

  it('bounds oldest-first state and preserves simultaneous exact repeats', () => {
    const repeated = damage({ tick: 9 });
    const next = reduceGmActivityFeed(
      { capacity: 99, entries: [destruction({ tick: 1 })] },
      payload([objective({ tick: 9 }), repeated, repeated, trigger({ tick: 9 })], 3),
    );
    expect(next.entries).toEqual([repeated, repeated, trigger({ tick: 9 })]);
  });

  it('composes category and semantic ship filters as AND and keeps global rows All-only', () => {
    const other = damage({
      tick: 2,
      ships: [reference(OTHER_SHIP, 'Other')],
      links: [{ role: 'victim', entity: reference(OTHER_SHIP, 'Other') }],
    });
    const entries = [damage({ tick: 1 }), destruction({ tick: 3 }), other, gmAction()];
    expect(filterGmActivityEntries(entries, { category: 'damage', ship: SHIP })
      .map(({ tick }) => tick)).toEqual([1]);
    expect(filterGmActivityEntries(entries, { ship: SHIP })
      .map(({ tick }) => tick)).toEqual([1, 3]);
    expect(filterGmActivityEntries(entries).map(({ category }) => category))
      .toContain('gm_action');
  });

  it('does not retain future presentation metadata', () => {
    const parsed = parseGmActivityFeed(payload([{ ...damage(), attention_score: 100 }]));
    expect(parsed.entries[0]).not.toHaveProperty('attention_score');
  });
});

describe('GM activity feed presentation and selection links', () => {
  let harness;

  beforeEach(() => {
    setTable(realStrings);
    harness = mount();
  });

  it('renders every category and exact operator outcome', () => {
    const rows = [...allCategories(), gmAction('no-op'), gmAction('refused')];
    expect(harness.feed.update(payload(rows, 16))).toBe(true);
    expect([...document.querySelectorAll('.gm-activity-entry')]
      .map((row) => row.dataset.category)).toEqual(rows.map(({ category }) => category));
    expect(document.querySelectorAll('[data-category="gm_action"]')).toHaveLength(3);
    expect(document.querySelector('[data-category="gm_action"]').textContent)
      .toContain('Morgan');
    expect(document.querySelector('[data-category="connection"]').textContent)
      .toContain('Ari');
  });

  it('drives category plus ship filters and clear resets both', () => {
    const other = damage({
      tick: 2,
      ships: [reference(OTHER_SHIP, 'Other')],
      links: [{ role: 'victim', entity: reference(OTHER_SHIP, 'Other') }],
    });
    harness.feed.update(payload([damage({ tick: 1 }), destruction({ tick: 3 }), other, gmAction()]));

    const category = document.getElementById('gm-activity-category-filter');
    const shipFilter = document.getElementById('gm-activity-ship-filter');
    category.value = 'damage';
    category.dispatchEvent(new Event('change'));
    shipFilter.value = SHIP;
    shipFilter.dispatchEvent(new Event('change'));
    expect([...document.querySelectorAll('.gm-activity-entry')].map((row) => row.dataset.tick))
      .toEqual(['1']);

    document.getElementById('gm-activity-clear-filters').click();
    expect(category.value).toBe('all');
    expect(shipFilter.value).toBe('all');
    expect(document.querySelectorAll('.gm-activity-entry')).toHaveLength(4);
  });

  it('resets a disappearing selected ship to All while retained links stay readable and disabled', () => {
    harness.feed.update(payload([damage(), gmAction()]));
    const shipFilter = document.getElementById('gm-activity-ship-filter');
    shipFilter.value = SHIP;
    shipFilter.dispatchEvent(new Event('change'));
    expect(document.querySelectorAll('.gm-activity-entry')).toHaveLength(1);

    harness.available.delete(SHIP);
    harness.feed.reconcileAvailability();
    expect(shipFilter.value).toBe('all');
    expect(document.querySelectorAll('.gm-activity-entry')).toHaveLength(2);
    const victim = document.querySelector('[data-involvement="victim"]');
    expect(victim.textContent).toBe(wireText('entity.alliance_cruiser.display_name'));
    expect(victim.disabled).toBe(true);
    victim.click();
    expect(harness.selectEntity).not.toHaveBeenCalled();
  });

  it('selects a live link and treats a failed racing selection as a no-op', () => {
    const available = new Set([SHIP, SOURCE]);
    const selection = vi.fn(() => {
      available.delete(SHIP);
      return false;
    });
    harness = mount({ available, selectEntity: selection });
    harness.feed.update(payload([damage()]));
    const victim = document.querySelector('[data-involvement="victim"]');
    victim.click();
    expect(selection).toHaveBeenCalledWith(SHIP);
    expect(victim.disabled).toBe(true);
  });
});

describe('GM activity transport separation', () => {
  it('uses only the page-local Host Channel and has a real server-page handler', () => {
    for (const file of [
      'src/core/messages.rs',
      'src/lockstep/frame.rs',
      'src/server_app/components.rs',
    ]) {
      const sourceText = read(file);
      expect(sourceText).not.toContain('GmActivityFeed');
      expect(sourceText).not.toContain('gm_activity');
    }

    const bridge = read('src/server/bridge.rs');
    const outbound = bridge.slice(
      bridge.indexOf('fn flush_outbound('),
      bridge.indexOf('fn flush_host_channels('),
    );
    expect(outbound).not.toContain('GmActivityFeed');
    expect(outbound).not.toContain('GM_ACTIVITY');
    expect(bridge).toContain('host_channels::GM_ACTIVITY');
    expect(read('server.html')).toContain('gm_activity:  function(p) { gmActivity.update(p); }');
  });
});
