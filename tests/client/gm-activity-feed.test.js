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

const VICTIM = '00000000-0000-4000-8000-000000000001';
const SOURCE = '00000000-0000-4000-8000-000000000002';

function reference(entity_id, name = entity_id) {
  return { entity_id, name };
}

function damage(overrides = {}) {
  return {
    tick: 7,
    category: 'damage',
    victim: reference(VICTIM, 'entity.alliance_cruiser.display_name'),
    source: reference(SOURCE, 'Raider'),
    damage: {
      victim_kind: 'ship',
      weapon: 'phaser-bank-1',
      amount: 4,
      shield_absorbed: 1,
      hull_damage: 3,
      system_hit: null,
    },
    ...overrides,
  };
}

function destroyed(overrides = {}) {
  return {
    tick: 7,
    category: 'destruction',
    victim: reference(VICTIM, 'entity.alliance_cruiser.display_name'),
    source: null,
    damage: null,
    ...overrides,
  };
}

function payload(entries, capacity = 128) {
  return { capacity, entries };
}

function mount({ available = new Set([VICTIM, SOURCE]), selectEntity = vi.fn(() => true) } = {}) {
  document.body.innerHTML = `
    <section id="gm-activity"><h2 id="gm-activity-heading"></h2>
      <select id="gm-activity-category-filter">
        <option value="all">all</option><option value="damage">damage</option>
        <option value="destruction">destruction</option>
      </select>
      <select id="gm-activity-identity-filter"></select>
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
  it('strictly narrows valid raw IDs and rejects category/detail mismatches', () => {
    const parsed = parseGmActivityFeed(JSON.stringify(payload([damage({
      internal_entity: 99,
      damage: { ...damage().damage, private_component: 'Hull' },
    })], 4)));
    expect(parsed).toEqual(payload([damage()], 4));
    expect(parsed.entries[0].victim.entity_id).toBe(VICTIM);
    expect(parsed.entries[0].damage.weapon).toBe('phaser-bank-1');
    expect(parsed.entries[0]).not.toHaveProperty('internal_entity');
    expect(parsed.entries[0].damage).not.toHaveProperty('private_component');

    expect(parseGmActivityFeed(payload([{ ...damage(), damage: null }]))).toBeUndefined();
    expect(parseGmActivityFeed(payload([{ ...destroyed(), damage: damage().damage }]))).toBeUndefined();
    expect(parseGmActivityFeed('{')).toBeUndefined();
  });

  it('bounds oldest-first absolute state and preserves exact repeated rows and order', () => {
    const repeated = damage();
    const next = reduceGmActivityFeed(
      { capacity: 99, entries: [destroyed({ tick: 1 })] },
      payload([
        damage({ tick: 4, victim: reference(SOURCE, 'first') }),
        repeated,
        repeated,
        destroyed(),
      ], 3),
    );
    expect(next.entries).toEqual([repeated, repeated, destroyed()]);
    expect(next.entries[0]).toEqual(next.entries[1]);
    expect(next.entries[2].category).toBe('destruction');
  });

  it('filters by category and either involved identity without reordering', () => {
    const entries = [damage({ tick: 1 }), destroyed({ tick: 2 }), damage({
      tick: 3,
      victim: reference('other', 'Other'),
      source: null,
    })];
    expect(filterGmActivityEntries(entries, { category: 'damage' }).map((e) => e.tick))
      .toEqual([1, 3]);
    expect(filterGmActivityEntries(entries, { identity: SOURCE }).map((e) => e.tick))
      .toEqual([1]);
    expect(filterGmActivityEntries(entries, { identity: VICTIM }).map((e) => e.tick))
      .toEqual([1, 2]);
  });
});

describe('GM activity feed presentation and selection links', () => {
  let harness;

  beforeEach(() => {
    setTable(realStrings);
    harness = mount();
  });

  it('localises known name IDs, preserves literal names, and uses UUID only when absent', () => {
    const unnamed = '00000000-0000-4000-8000-000000000004';
    harness.feed.update(payload([
      damage(),
      destroyed({ tick: 8, victim: reference(unnamed, '') }),
    ], 4));

    const victims = [...document.querySelectorAll('[data-involvement="victim"]')];
    expect(victims[0].textContent).toBe(wireText('entity.alliance_cruiser.display_name'));
    expect(document.querySelector('[data-involvement="source"]').textContent).toBe('Raider');
    expect(victims[1].textContent).toBe(unnamed);
  });

  it('renders repeats, drives category/identity filters, and selects live map identity', () => {
    expect(harness.feed.update(payload([damage(), damage(), destroyed()], 3))).toBe(true);
    expect(document.querySelectorAll('.gm-activity-entry')).toHaveLength(3);
    expect(document.querySelectorAll('[data-involvement="victim"]')).toHaveLength(3);

    document.querySelector('[data-involvement="victim"]').click();
    expect(harness.selectEntity).toHaveBeenCalledWith(VICTIM);

    const category = document.getElementById('gm-activity-category-filter');
    category.value = 'destruction';
    category.dispatchEvent(new Event('change'));
    expect([...document.querySelectorAll('.gm-activity-entry')]
      .map((row) => row.dataset.category)).toEqual(['destruction']);

    category.value = 'all';
    category.dispatchEvent(new Event('change'));
    const identity = document.getElementById('gm-activity-identity-filter');
    identity.value = SOURCE;
    identity.dispatchEvent(new Event('change'));
    expect(document.querySelectorAll('.gm-activity-entry')).toHaveLength(2);
  });

  it('keeps a removed identity readable and disabled and a racing click cannot clear selection', () => {
    harness.feed.update(payload([damage()], 4));
    const victim = document.querySelector('[data-involvement="victim"]');
    expect(victim.textContent).toBe(wireText('entity.alliance_cruiser.display_name'));
    expect(victim.disabled).toBe(false);

    harness.available.delete(VICTIM);
    harness.feed.reconcileAvailability();
    expect(victim.textContent).toBe(wireText('entity.alliance_cruiser.display_name'));
    expect(victim.disabled).toBe(true);
    victim.click();
    expect(harness.selectEntity).not.toHaveBeenCalled();
  });

  it('treats a failed live selection as a no-op', () => {
    const selection = vi.fn(() => false);
    harness = mount({ selectEntity: selection });
    harness.feed.update(payload([damage()], 4));
    document.querySelector('[data-involvement="victim"]').click();
    expect(selection).toHaveBeenCalledWith(VICTIM);
    expect(document.querySelector('[data-involvement="victim"]').disabled).toBe(false);
  });
});

describe('GM activity transport separation', () => {
  it('uses only the page-local Host Channel and has a real server-page handler', () => {
    for (const file of [
      'src/core/messages.rs',
      'src/lockstep/frame.rs',
      'src/server_app/components.rs',
    ]) {
      const source = read(file);
      expect(source).not.toContain('GmActivityFeed');
      expect(source).not.toContain('gm_activity');
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
