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

it('keeps Comms Applied and Refused outcomes under each captured recipient filter', () => {
  const recipients = [ship, reference(OTHER_SHIP, 'Departed recipient')];
  const entries = ['applied', 'refused'].map(outcome => entry('gm_action', {
    type: 'gm_action', data: {
      operator: { id: 'gm-alpha', name: 'Morgan' }, correlation: `comms-${outcome}`,
      action: { type: 'transmit_comms', sender: SOURCE }, outcome,
      reason: outcome === 'refused' ? 'unavailable-comms-recipient' : null,
      order: { sequence: outcome === 'applied' ? 1 : 2, origin: 1 },
    },
  }, { ships: recipients, links: recipients.map(entity => ({ role: 'ship', entity })) }));
  const parsed = parseGmActivityFeed(JSON.stringify({ capacity: 32, entries }));
  expect(parsed).toBeDefined();
  for (const recipient of recipients) {
    const filtered = filterGmActivityEntries(parsed.entries, { category: 'gm_action', ship: recipient.entity_id });
    expect(filtered.map(row => row.detail.data.outcome)).toEqual(['applied', 'refused']);
  }
  expect(filterGmActivityEntries(parsed.entries, { ship: SOURCE })).toEqual([]);
});

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

/** One Fire of an authored GM event (issue #1301), the third action family. */
function fireGmEvent(data = {}, action = {}) {
  return entry('gm_action', {
    type: 'gm_action',
    data: {
      operator: { id: 'gm-alpha', name: 'Morgan' },
      correlation: 'fire-1',
      action: { type: 'fire_gm_event', event: 'base-world::breach_alarm', ...action },
      outcome: 'applied',
      reason: null,
      order: { sequence: 5, origin: 1 },
      ...data,
    },
  }, { ships: [], links: [] });
}

/** One Skip-next arm on an authored GM event (issue #1304): the Fire's twin. */
function armGmEventSkip(data = {}, action = {}) {
  return entry('gm_action', {
    type: 'gm_action',
    data: {
      operator: { id: 'gm-alpha', name: 'Morgan' },
      correlation: 'skip-1',
      action: { type: 'arm_gm_event_skip', event: 'base-world::courier_lost', ...action },
      outcome: 'applied',
      reason: null,
      order: { sequence: 6, origin: 1 },
      ...data,
    },
  }, { ships: [], links: [] });
}

/** One directed world effect (issue #1310), the fourth action family. */
function applyDirectEffect(data = {}, action = {}) {
  return entry('gm_action', {
    type: 'gm_action',
    data: {
      operator: { id: 'gm-alpha', name: 'Morgan' },
      correlation: 'hit-1',
      action: {
        type: 'apply_direct_effect',
        entity: 'npc-1',
        heal: false,
        applied_milli_hp: 25000,
        discarded_milli_hp: 0,
        destroyed: false,
        ...action,
      },
      outcome: 'applied',
      reason: null,
      order: { sequence: 6, origin: 1 },
      ...data,
    },
  }, { ships: [], links: [] });
}

/** One palette placement (issue #1305), the fifth action family. */
function spawnPaletteEntity(data = {}, action = {}) {
  return entry('gm_action', {
    type: 'gm_action',
    data: {
      operator: { id: 'gm-alpha', name: 'Morgan' },
      correlation: 'place-1',
      action: { type: 'spawn_palette_entity', palette: 'raider', ...action },
      outcome: 'applied',
      reason: null,
      order: { sequence: 7, origin: 1 },
      ...data,
    },
  }, { ships: [], links: [] });
}

/** One Pause/Resume of an authored GM event (issue #1303), the same family. */
function pauseGmEvent(data = {}, action = {}) {
  return entry('gm_action', {
    type: 'gm_action',
    data: {
      operator: { id: 'gm-alpha', name: 'Morgan' },
      correlation: 'pause-event-1',
      action: {
        type: 'set_event_paused',
        event: 'base-world::breach_alarm',
        active: true,
        ...action,
      },
      outcome: 'applied',
      reason: null,
      order: { sequence: 8, origin: 1 },
      ...data,
    },
  }, { ships: [], links: [] });
}

/** One ordered faction pair's hostility (issue #1442), the typed adapter. */
function setFactionHostility(data = {}, action = {}) {
  return entry('gm_action', {
    type: 'gm_action',
    data: {
      operator: { id: 'gm-alpha', name: 'Morgan' },
      correlation: 'faction-1',
      action: {
        type: 'set_faction_hostility',
        faction: 'Alliance',
        enemy: 'Harrow',
        hostile: true,
        ...action,
      },
      outcome: 'applied',
      reason: null,
      order: { sequence: 9, origin: 1 },
      ...data,
    },
  }, { ships: [], links: [] });
}

/** One equal GM's reversal of another's earlier action (issue #1442). */
function undoGmAction(data = {}, action = {}) {
  return entry('gm_action', {
    type: 'gm_action',
    data: {
      operator: { id: 'gm-beta', name: 'Blake' },
      correlation: 'undo-1',
      action: {
        type: 'undo_gm_action',
        original_operator: { id: 'gm-alpha', name: 'Morgan' },
        original_correlation: 'faction-1',
        ...action,
      },
      outcome: 'applied',
      reason: null,
      order: { sequence: 10, origin: 1 },
      ...data,
    },
  }, { ships: [], links: [] });
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

  it('accepts a fired GM event and rejects a nameless one', () => {
    const parsed = parseGmActivityFeed(payload([fireGmEvent()]));
    expect(parsed.entries).toHaveLength(1);
    expect(parsed.entries[0].detail.data.action)
      .toEqual({ type: 'fire_gm_event', event: 'base-world::breach_alarm' });
    // Strictness is payload-wide and deliberate: an unnamed or unknown action
    // rejects the whole absolute page rather than rendering a wrong sentence.
    // That is exactly why the browser must carry the same action vocabulary
    // Rust publishes — a variant known to only one side freezes the feed.
    expect(parseGmActivityFeed(payload([fireGmEvent({}, { event: '' })]))).toBeUndefined();
    expect(parseGmActivityFeed(payload([fireGmEvent({}, { type: 'fire_unknown' })])))
      .toBeUndefined();
  });

  it('preserves each Objective verb and intended recipients and refuses malformed scope', () => {
    for (const verb of ['activate', 'complete', 'fail']) {
      const action = { type: 'objective_action', objective: 'rescue', verb, recipients: [SHIP] };
      const row = fireGmEvent({}, action);
      expect(parseGmActivityFeed(payload([row])).entries[0].detail.data.action).toEqual(action);
      for (const invalid of [{ recipients: null }, { recipients: [SHIP, SHIP] }, { objective: '' }, { verb: 'reopen' }]) {
        expect(parseGmActivityFeed(payload([fireGmEvent({}, { ...action, ...invalid })]))).toBeUndefined();
      }
    }
  });

  it('keeps scoped Objective results under their semantic ship filter, including refusals', () => {
    const rows = ['activate', 'complete', 'fail'].flatMap((verb) => ['applied', 'refused'].map((outcome) => {
      const action = { type: 'objective_action', objective: 'rescue', verb, recipients: [SHIP] };
      const row = fireGmEvent({ outcome, reason: outcome === 'refused' ? 'objective-scope-mismatch' : null });
      row.detail.data.action = action;
      return { ...row,
        ships: [ship], links: [{ role: 'ship', entity: ship }] };
    }));
    const global = fireGmEvent({}, { type: 'objective_action', objective: 'global', verb: 'activate', recipients: [] });
    const parsed = parseGmActivityFeed(payload([...rows, global]));
    expect(filterGmActivityEntries(parsed.entries, { category: 'gm_action', ship: SHIP })).toEqual(rows);
    expect(filterGmActivityEntries(parsed.entries, { ship: OTHER_SHIP })).toEqual([]);
    expect(filterGmActivityEntries(parsed.entries, { ship: 'all' })).toHaveLength(7);
  });

  it('accepts an armed Skip and keeps it distinct from a Fire', () => {
    const parsed = parseGmActivityFeed(payload([armGmEventSkip()]));
    expect(parsed.entries).toHaveLength(1);
    expect(parsed.entries[0].detail.data.action)
      .toEqual({ type: 'arm_gm_event_skip', event: 'base-world::courier_lost' });
    expect(parseGmActivityFeed(payload([armGmEventSkip({}, { event: '' })])))
      .toBeUndefined();
  });

  it('accepts a directed world effect and rejects a malformed one', () => {
    const parsed = parseGmActivityFeed(payload([applyDirectEffect()]));
    expect(parsed.entries).toHaveLength(1);
    expect(parsed.entries[0].detail.data.action).toEqual({
      type: 'apply_direct_effect',
      entity: 'npc-1',
      heal: false,
      applied_milli_hp: 25000,
      discarded_milli_hp: 0,
      destroyed: false,
    });
    for (const broken of [
      { entity: '' },
      { heal: 'yes' },
      { applied_milli_hp: -1 },
      { discarded_milli_hp: 1.5 },
      { destroyed: null },
      // A malformed narrowing rejects the row rather than widening it to the
      // whole ship (issue #1311).
      { scope: { deck: 'a' } },
      { scope: { station: '' } },
      { scope: 'station' },
    ]) {
      expect(parseGmActivityFeed(payload([applyDirectEffect({}, broken)]))).toBeUndefined();
    }
  });

  it('carries the Station or System a narrowed effect was aimed at', () => {
    for (const [wire, parsed] of [
      [{ station: 'helm' }, { kind: 'station', id: 'helm' }],
      [{ system: 'impulse-drive' }, { kind: 'system', id: 'impulse-drive' }],
    ]) {
      const feed = parseGmActivityFeed(payload([applyDirectEffect({}, { scope: wire })]));
      expect(feed.entries[0].detail.data.action.scope).toEqual(parsed);
    }
    // The whole-hull spelling carries no scope at all, which is what every
    // pre-#1311 row meant, so those rows parse byte for byte as they did.
    const whole = parseGmActivityFeed(payload([applyDirectEffect()]));
    expect(whole.entries[0].detail.data.action.scope).toBeUndefined();
    const explicit = parseGmActivityFeed(
      payload([applyDirectEffect({}, { scope: 'entity' })]),
    );
    expect(explicit.entries[0].detail.data.action.scope).toBeUndefined();
  });

  it('accepts a palette placement and rejects a nameless one', () => {
    const parsed = parseGmActivityFeed(payload([spawnPaletteEntity()]));
    expect(parsed.entries).toHaveLength(1);
    expect(parsed.entries[0].detail.data.action)
      .toEqual({ type: 'spawn_palette_entity', palette: 'raider' });
    // Same payload-wide strictness the Fire family gets, and the same reason:
    // the browser carries the exact action vocabulary Rust publishes.
    expect(parseGmActivityFeed(payload([spawnPaletteEntity({}, { palette: '' })])))
      .toBeUndefined();
    // And no template path can ride in: the placement row names an id only.
    expect(parsed.entries[0].detail.data.action).not.toHaveProperty('template_path');
  });

  it('accepts a paused GM event and tells it apart from a fired one', () => {
    const parsed = parseGmActivityFeed(payload([pauseGmEvent(), pauseGmEvent({}, {
      active: false,
    })]));
    expect(parsed.entries.map((row) => row.detail.data.action)).toEqual([
      { type: 'set_event_paused', event: 'base-world::breach_alarm', active: true },
      { type: 'set_event_paused', event: 'base-world::breach_alarm', active: false },
    ]);
    // The absolute state is required: a Pause row without it could only be
    // rendered by guessing which position of the toggle was asked for.
    expect(parseGmActivityFeed(payload([pauseGmEvent({}, { active: 'yes' })])))
      .toBeUndefined();
    expect(parseGmActivityFeed(payload([pauseGmEvent({}, { event: '' })]))).toBeUndefined();
  });

  it('accepts a faction relation and an attributed undo without a pause fallback', () => {
    const parsed = parseGmActivityFeed(payload([
      setFactionHostility(),
      // A refusal moved no pair, so it records the faction alone.
      setFactionHostility(
        { correlation: 'faction-2', outcome: 'refused', reason: 'unknown-faction' },
        { enemy: undefined, hostile: false },
      ),
      undoGmAction(),
    ]));
    expect(parsed.entries.map((row) => row.detail.data.action)).toEqual([
      {
        type: 'set_faction_hostility', faction: 'Alliance', enemy: 'Harrow', hostile: true,
      },
      { type: 'set_faction_hostility', faction: 'Alliance', hostile: false },
      {
        type: 'undo_gm_action',
        original_operator: { id: 'gm-alpha', name: 'Morgan' },
        original_correlation: 'faction-1',
      },
    ]);
    // Neither family is the absolute session toggle, so neither may parse into
    // one: `active` is not part of the contract and cannot ride along.
    for (const row of parsed.entries) {
      expect(row.detail.data.action).not.toHaveProperty('active');
    }
    // The same payload-wide strictness every other family gets.
    expect(parseGmActivityFeed(payload([setFactionHostility({}, { faction: '' })])))
      .toBeUndefined();
    expect(parseGmActivityFeed(payload([setFactionHostility({}, { hostile: 'yes' })])))
      .toBeUndefined();
    expect(parseGmActivityFeed(payload([setFactionHostility({}, { enemy: '' })])))
      .toBeUndefined();
    // An undo that cannot name WHOSE action it reversed is rejected rather
    // than rendered half-attributed: both operators are the point of the row.
    expect(parseGmActivityFeed(payload([undoGmAction({}, { original_operator: undefined })])))
      .toBeUndefined();
    expect(parseGmActivityFeed(payload([undoGmAction({}, {
      original_operator: { id: '', name: 'Morgan' },
    })]))).toBeUndefined();
    expect(parseGmActivityFeed(payload([undoGmAction({}, { original_correlation: '' })])))
      .toBeUndefined();
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

  it('names the fired event and its refusal reason from the String Table', () => {
    const refused = fireGmEvent({
      correlation: 'fire-2',
      outcome: 'refused',
      reason: 'unknown-gm-event',
    });
    expect(harness.feed.update(payload([fireGmEvent(), refused], 16))).toBe(true);
    const rows = [...document.querySelectorAll('[data-category="gm_action"]')];
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent)
      .toContain('server.gm.activity.action.fire_gm_event:base-world::breach_alarm');
    expect(rows[0].textContent).toContain('server.gm.activity.action_outcome.applied');
    expect(rows[1].textContent).toContain('server.gm.activity.action_reason.unknown-gm-event');
    // A Fire must never borrow the pause family's sentence.
    for (const row of rows) {
      expect(row.textContent).not.toContain('server.gm.activity.action.resume');
    }
    // And both ids this branch composes resolve against the shipped table.
    expect(realStrings.get('server.gm.activity.action.fire_gm_event')).toContain('{event}');
    expect(realStrings.has('server.gm.activity.action_reason.unknown-gm-event')).toBe(true);
  });

  it('says a Skip was armed, never that the event was fired', () => {
    expect(harness.feed.update(payload([fireGmEvent(), armGmEventSkip()], 16))).toBe(true);
    const rows = [...document.querySelectorAll('[data-category="gm_action"]')];
    expect(rows).toHaveLength(2);
    const sentences = rows.map((row) => row.textContent).join(' | ');
    expect(sentences)
      .toContain('server.gm.activity.action.arm_gm_event_skip:base-world::courier_lost');
    expect(sentences)
      .toContain('server.gm.activity.action.fire_gm_event:base-world::breach_alarm');
    // The two levers do opposite things: neither may borrow the other's
    // sentence, and neither may fall back to the pause family's.
    const skipRow = rows.find((row) => row.textContent.includes('courier_lost'));
    expect(skipRow.textContent).not.toContain('action.fire_gm_event');
    expect(skipRow.textContent).not.toContain('server.gm.activity.action.resume');
    expect(realStrings.get('server.gm.activity.action.arm_gm_event_skip')).toContain('{event}');
  });

  it('names the damaged entity, the hull that landed and the lethal outcome', () => {
    const lethal = applyDirectEffect({ correlation: 'hit-2' }, {
      applied_milli_hp: 30000,
      discarded_milli_hp: 60000,
      destroyed: true,
    });
    const heal = applyDirectEffect({ correlation: 'heal-1' }, {
      heal: true,
      applied_milli_hp: 5500,
    });
    expect(harness.feed.update(payload([applyDirectEffect(), lethal, heal], 16))).toBe(true);
    const rows = [...document.querySelectorAll('[data-category="gm_action"]')];
    expect(rows).toHaveLength(3);
    expect(rows[0].textContent)
      .toContain('server.gm.activity.action.apply_direct_damage:npc-1/25');
    expect(rows[1].textContent).toContain('server.gm.activity.action.direct_effect_lethal');
    expect(rows[1].textContent)
      .toContain('server.gm.activity.action.direct_effect_discarded:60');
    // Healing is its own sentence, and a fractional amount survives the unit
    // conversion rather than being rounded to a whole hull point.
    expect(rows[2].textContent)
      .toContain('server.gm.activity.action.apply_direct_heal:npc-1/5.5');
    for (const row of rows) {
      expect(row.textContent).not.toContain('server.gm.activity.action.resume');
    }
    expect(realStrings.get('server.gm.activity.action.apply_direct_damage')).toContain('{entity}');
    expect(realStrings.get('server.gm.activity.action.apply_direct_heal')).toContain('{amount}');
  });

  it('names the Station or System a narrowed effect emptied (issue #1311)', () => {
    const station = applyDirectEffect({ correlation: 'hit-helm' }, {
      scope: { station: 'helm' },
      applied_milli_hp: 20000,
    });
    const system = applyDirectEffect({ correlation: 'heal-drive' }, {
      scope: { system: 'impulse-drive' },
      heal: true,
      applied_milli_hp: 4000,
    });
    expect(harness.feed.update(payload([applyDirectEffect(), station, system], 16))).toBe(true);
    const rows = [...document.querySelectorAll('[data-category="gm_action"]')];
    expect(rows).toHaveLength(3);
    // A whole-hull row still says nothing about a scope, which is what makes
    // the narrowed rows readable as narrowed.
    expect(rows[0].textContent)
      .not.toContain('server.gm.activity.action.direct_effect_station');
    expect(rows[1].textContent)
      .toContain('server.gm.activity.action.direct_effect_station:helm');
    expect(rows[2].textContent)
      .toContain('server.gm.activity.action.direct_effect_system:impulse-drive');
    // And both ids this branch composes resolve against the shipped table.
    expect(realStrings.get('server.gm.activity.action.direct_effect_station'))
      .toContain('{scope}');
    expect(realStrings.get('server.gm.activity.action.direct_effect_system'))
      .toContain('{scope}');
  });

  it('names the placed palette entry and its refusal reason from the String Table', () => {
    const refused = spawnPaletteEntity({
      correlation: 'place-2',
      outcome: 'refused',
      reason: 'unknown-gm-palette-entry',
    });
    expect(harness.feed.update(payload([spawnPaletteEntity(), refused], 16))).toBe(true);
    const rows = [...document.querySelectorAll('[data-category="gm_action"]')];
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent)
      .toContain('server.gm.activity.action.spawn_palette_entity:raider');
    expect(rows[0].textContent).toContain('server.gm.activity.action_outcome.applied');
    expect(rows[1].textContent)
      .toContain('server.gm.activity.action_reason.unknown-gm-palette-entry');
    // A placement must never borrow the pause family's sentence.
    for (const row of rows) {
      expect(row.textContent).not.toContain('server.gm.activity.action.resume');
    }
    expect(realStrings.get('server.gm.activity.action.spawn_palette_entity'))
      .toContain('{palette}');
    expect(realStrings.has('server.gm.activity.action_reason.unknown-gm-palette-entry'))
      .toBe(true);
  });

  it('names the moved faction pair and both operators of an undo (issue #1442)', () => {
    const withdrawn = setFactionHostility({ correlation: 'faction-2' }, { hostile: false });
    const refused = setFactionHostility(
      { correlation: 'faction-3', outcome: 'refused', reason: 'unknown-faction' },
      { enemy: undefined },
    );
    expect(harness.feed.update(payload(
      [setFactionHostility(), withdrawn, refused, undoGmAction()],
      16,
    ))).toBe(true);
    const rows = [...document.querySelectorAll('[data-category="gm_action"]')];
    expect(rows).toHaveLength(4);
    expect(rows[0].textContent)
      .toContain('server.gm.activity.action.faction_hostile:Alliance/Harrow');
    // Standing a hostility down is its own sentence, not the same one negated.
    expect(rows[1].textContent)
      .toContain('server.gm.activity.action.faction_friendly:Alliance/Harrow');
    // A refusal moved no pair, so it names the faction alone rather than
    // claiming a relation against an enemy nobody was named against.
    expect(rows[2].textContent)
      .toContain('server.gm.activity.action.faction_hostile_unnamed:Alliance/');
    expect(rows[2].textContent)
      .toContain('server.gm.activity.action_reason.unknown-faction');
    // Both operators on the one shared row: the undoing GM is the row's own
    // operator and the one whose action was reversed is named in the sentence.
    expect(rows[3].textContent)
      .toContain('server.gm.activity.action.undo_gm_action:Morgan/faction-1');
    expect(rows[3].textContent).toContain('Blake');
    // The bug this pins: before these families had their own arms they fell
    // through to the catch-all and every GM read "Morgan paused the session"
    // for a faction change or an undo.
    for (const row of rows) {
      expect(row.textContent).not.toContain('server.gm.activity.action.pause:');
      expect(row.textContent).not.toContain('server.gm.activity.action.resume:');
    }
    expect(realStrings.get('server.gm.activity.action.faction_hostile')).toContain('{enemy}');
    expect(realStrings.get('server.gm.activity.action.faction_friendly')).toContain('{enemy}');
    expect(realStrings.get('server.gm.activity.action.faction_hostile_unnamed'))
      .toContain('{faction}');
    expect(realStrings.get('server.gm.activity.action.faction_friendly_unnamed'))
      .toContain('{faction}');
    expect(realStrings.get('server.gm.activity.action.undo_gm_action')).toContain('{operator}');
    for (const reason of [
      'unknown-faction', 'unknown-gm-action', 'inverse-unsupported',
      'inverse-facts-mismatch', 'affected-state-changed', 'already-inverted',
    ]) {
      expect(realStrings.has(`server.gm.activity.action_reason.${reason}`)).toBe(true);
    }
  });

  it('names Objective actions with scope and refusal copy without borrowing an event verb', () => {
    const rows = ['activate', 'complete', 'fail'].map((verb, index) => fireGmEvent({
      correlation: `objective-${index}`, outcome: 'refused', reason: 'objective-scope-mismatch',
    }, { type: 'objective_action', objective: 'rescue', verb, recipients: [SHIP] }));
    expect(harness.feed.update(payload(rows, 16))).toBe(true);
    const rendered = [...document.querySelectorAll('[data-category="gm_action"]')];
    expect(rendered).toHaveLength(3);
    for (const [index, verb] of ['activate', 'complete', 'fail'].entries()) {
      expect(rendered[index].textContent).toContain(`server.gm.activity.action.objective_${verb}`);
      expect(realStrings.get(`server.gm.activity.action.objective_${verb}`)).toContain('{ships}');
    }
    expect(realStrings.has('server.gm.activity.action_reason.objective-scope-mismatch')).toBe(true);
  });

  it('says paused and resumed rather than fired for the same event', () => {
    expect(harness.feed.update(payload([
      pauseGmEvent(),
      pauseGmEvent({ correlation: 'resume-event-1' }, { active: false }),
    ], 16))).toBe(true);
    const rows = [...document.querySelectorAll('[data-category="gm_action"]')];
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent)
      .toContain('server.gm.activity.action.pause_gm_event:base-world::breach_alarm');
    expect(rows[1].textContent)
      .toContain('server.gm.activity.action.resume_gm_event:base-world::breach_alarm');
    // The bug this whole branch exists to prevent: Pause shares one
    // `GmActionKind` with Fire, so a feed folding on the kind alone renders
    // both of these as a fire of the same event.
    for (const row of rows) {
      expect(row.textContent).not.toContain('server.gm.activity.action.fire_gm_event');
    }
    expect(realStrings.get('server.gm.activity.action.pause_gm_event')).toContain('{event}');
    expect(realStrings.get('server.gm.activity.action.resume_gm_event')).toContain('{event}');
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

  it('filters contact actions by their observer and retains historical refused links', () => {
    const contact = (outcome, index, observer = ship) => entry('gm_action', {
      type: 'gm_action', data: {
        operator: { id: 'gm-alpha', name: 'Morgan' }, correlation: `contact-${index}`,
        action: { type: 'set_contact_override', observer: observer.entity_id,
          target: SOURCE, mode: index === 2 ? 'conceal' : 'reveal' },
        outcome, reason: outcome === 'refused' ? 'unknown-entity' : null,
        order: { sequence: index + 1, origin: 1 },
      },
    }, { tick: index + 1, ships: [observer], links: [{ role: 'ship', entity: observer }] });
    const rows = [contact('applied', 0), contact('no-op', 1),
      contact('refused', 2, reference(SHIP)),
      contact('applied', 3, reference(OTHER_SHIP, 'Observer B'))];
    expect(harness.feed.update(payload(rows))).toBe(true);
    const category = document.getElementById('gm-activity-category-filter');
    const shipFilter = document.getElementById('gm-activity-ship-filter');
    category.value = 'gm_action'; category.dispatchEvent(new Event('change'));
    shipFilter.value = SHIP; shipFilter.dispatchEvent(new Event('change'));
    expect([...document.querySelectorAll('.gm-activity-entry')].map(row => row.dataset.tick))
      .toEqual(['1', '2', '3']);
    document.querySelector('[data-involvement="ship"]').click();
    expect(harness.selectEntity).toHaveBeenCalledWith(SHIP);
    shipFilter.value = OTHER_SHIP; shipFilter.dispatchEvent(new Event('change'));
    expect([...document.querySelectorAll('.gm-activity-entry')].map(row => row.dataset.tick))
      .toEqual(['4']);
    shipFilter.value = SHIP; shipFilter.dispatchEvent(new Event('change'));
    harness.available.delete(SHIP); harness.feed.reconcileAvailability();
    expect(shipFilter.value).toBe('all');
    const refused = document.querySelector('.gm-activity-entry[data-tick="3"]');
    const historicLink = refused.querySelector('[data-involvement="ship"]');
    expect(historicLink.disabled).toBe(true);
    expect(historicLink.textContent).toBe(SHIP);
    expect(document.querySelectorAll('.gm-activity-entry')).toHaveLength(4);
  });

  it('renders and filters System availability facts with their exact target and System', () => {
    const rows = ['applied', 'no-op', 'refused'].map((outcome, index) => entry('gm_action', {
      type: 'gm_action', data: {
        operator: { id: 'gm-alpha', name: 'Morgan' }, correlation: `system-${index}`,
        action: { type: 'set_system_disabled', target: SHIP, system: 'impulse-drive', disabled: index < 2 },
        outcome, reason: outcome === 'refused' ? 'unknown-system' : null,
        order: { sequence: index + 1, origin: 1 },
      },
    }, { tick: index + 1, ships: [ship], links: [{ role: 'ship', entity: ship }] }));
    expect(harness.feed.update(payload(rows))).toBe(true);
    const filter = document.getElementById('gm-activity-ship-filter');
    filter.value = SHIP; filter.dispatchEvent(new Event('change'));
    expect(document.querySelectorAll('.gm-activity-entry')).toHaveLength(3);
    expect(document.querySelector('.gm-activity-entry').textContent).toContain('impulse-drive');
    document.querySelector('[data-involvement="ship"]').click();
    expect(harness.selectEntity).toHaveBeenCalledWith(SHIP);
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
  it('retains attributed NPC doctrine rows under the semantic ship filter', () => {
    setTable(realStrings);
    const npc = entry('gm_action', { type: 'gm_action', data: { operator: { id: 'gm-alpha', name: 'Morgan' }, correlation: 'npc-1',
      action: { type: 'set_npc_doctrine', target: SHIP, doctrine: 'north' }, outcome: 'applied', reason: null, order: { sequence: 9, origin: 1 } } });
    const parsed = parseGmActivityFeed(payload([npc]));
    expect(parsed.entries[0].detail.data.action).toEqual({ type: 'set_npc_doctrine', target: SHIP, doctrine: 'north' });
    expect(filterGmActivityEntries(parsed.entries, { category: 'gm_action', ship: SHIP })).toHaveLength(1);
    expect(filterGmActivityEntries(parsed.entries, { category: 'gm_action', ship: OTHER_SHIP })).toHaveLength(0);
  });
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
    expect(read('gui/gm-workspace.js')).toContain('gm_activity:  function(p) { gmActivity.update(p); }');
  });
});
