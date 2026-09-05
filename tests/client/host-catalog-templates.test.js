import { describe, it, expect } from 'vitest';
import {
  catalogTemplatePaths,
  deliverCatalogTemplates,
  MAX_INCLUDE_ROUNDS,
} from '../../gui/host-catalog-templates.js';

const CATALOG = [
  {
    id: 'combat_test',
    ships: [
      { template_path: 'assets/entities/alliance_destroyer.toml', label: 'Destroyer' },
      { template_path: 'assets/entities/alliance_cruiser.toml', label: 'Cruiser' },
    ],
  },
  {
    id: 'falling_skyway',
    ships: [
      // Shared with the row above: one fetch, not two.
      { template_path: 'assets/entities/alliance_destroyer.toml', label: 'Destroyer' },
      { template_path: 'assets/entities/alliance_courier.toml', label: 'Courier' },
    ],
  },
];

describe('catalogTemplatePaths', () => {
  it('collects each distinct hull path once, in first-seen order', () => {
    expect(catalogTemplatePaths(CATALOG)).toEqual([
      'assets/entities/alliance_destroyer.toml',
      'assets/entities/alliance_cruiser.toml',
      'assets/entities/alliance_courier.toml',
    ]);
  });

  it('is empty for a catalog with no scenarios, no ships, or unnamed ships', () => {
    expect(catalogTemplatePaths(null)).toEqual([]);
    expect(catalogTemplatePaths([])).toEqual([]);
    expect(catalogTemplatePaths([{ id: 'a' }])).toEqual([]);
    expect(catalogTemplatePaths([{ id: 'a', ships: [{ label: 'no path' }, { template_path: '' }] }]))
      .toEqual([]);
  });
});

/** A fetcher over a fixture map; anything absent 404s. */
function fetcherOver(files) {
  const requested = [];
  return {
    requested,
    fetchText: async (path) => {
      requested.push(path);
      if (!(path in files)) throw new Error('HTTP 404');
      return files[path];
    },
  };
}

/** A push that answers with whatever fragments the fixture says are missing. */
function pusherOver(missingByPath) {
  const pushed = [];
  return {
    pushed,
    pushTemplate: (path, toml, isRoot) => {
      pushed.push({ path, toml, isRoot });
      const delivered = new Set(pushed.map((p) => p.path));
      const outstanding = [];
      for (const entry of pushed) {
        for (const frag of missingByPath[entry.path] || []) {
          if (!delivered.has(frag)) outstanding.push(frag);
        }
      }
      return outstanding;
    },
  };
}

describe('deliverCatalogTemplates', () => {
  it('delivers every root in one round when nothing includes anything', async () => {
    const files = { a: 'A', b: 'B' };
    const f = fetcherOver(files);
    const p = pusherOver({});

    const report = await deliverCatalogTemplates({
      paths: ['a', 'b'],
      fetchText: f.fetchText,
      pushTemplate: p.pushTemplate,
    });

    expect(report.delivered.sort()).toEqual(['a', 'b']);
    expect(report.unfetched).toEqual([]);
    expect(report.failed).toEqual([]);
    expect(report.rounds).toBe(1);
    expect(report.truncated).toBe(false);
    expect(p.pushed.every((x) => x.isRoot)).toBe(true);
    expect(p.pushed.map((x) => x.toml).sort()).toEqual(['A', 'B']);
  });

  it('fetches a round concurrently rather than serially', async () => {
    let inFlight = 0;
    let peak = 0;
    const report = await deliverCatalogTemplates({
      paths: ['a', 'b', 'c', 'd'],
      fetchText: async () => {
        inFlight += 1;
        peak = Math.max(peak, inFlight);
        await Promise.resolve();
        await Promise.resolve();
        inFlight -= 1;
        return 'toml';
      },
      pushTemplate: () => [],
    });

    expect(report.delivered).toHaveLength(4);
    expect(peak).toBe(4);
  });

  it('follows an include closure in further rounds, marked as non-root', async () => {
    const files = { hull: 'H', 'frag/a': 'FA', 'frag/b': 'FB' };
    const f = fetcherOver(files);
    const p = pusherOver({ hull: ['frag/a'], 'frag/a': ['frag/b'] });

    const report = await deliverCatalogTemplates({
      paths: ['hull'],
      fetchText: f.fetchText,
      pushTemplate: p.pushTemplate,
    });

    expect(report.delivered).toEqual(['hull', 'frag/a', 'frag/b']);
    expect(report.rounds).toBe(3);
    expect(report.truncated).toBe(false);
    expect(p.pushed.map((x) => x.isRoot)).toEqual([true, false, false]);
  });

  it('fetches a path shared by two roots exactly once', async () => {
    const files = { one: '1', two: '2', shared: 'S' };
    const f = fetcherOver(files);
    const p = pusherOver({ one: ['shared'], two: ['shared'] });

    await deliverCatalogTemplates({
      paths: ['one', 'two'],
      fetchText: f.fetchText,
      pushTemplate: p.pushTemplate,
    });

    expect(f.requested.filter((x) => x === 'shared')).toEqual(['shared']);
  });

  // The mod-pack case is the reason this is not simply \"drop it\": a pack's own
  // hull lives in the session overlay and has NO URL, so its fetch always 404s
  // and Rust's overlay lookup is the only thing that can supply its text. The
  // same contract `handleConfigRequest` keeps with `wasm_load_config` on a 404.
  it('still delivers a hull whose fetch brought nothing, as the empty string', async () => {
    const f = fetcherOver({ good: 'G' });
    const p = pusherOver({});

    const report = await deliverCatalogTemplates({
      paths: ['good', 'overlay-only'],
      fetchText: f.fetchText,
      pushTemplate: p.pushTemplate,
    });

    expect(report.delivered).toEqual(['good']);
    expect(report.unfetched).toEqual(['overlay-only']);
    expect(report.failed).toEqual([]);
    expect(p.pushed.map((x) => x.path).sort()).toEqual(['good', 'overlay-only']);
    expect(p.pushed.find((x) => x.path === 'overlay-only'))
      .toMatchObject({ toml: '', isRoot: true });
  });

  // ...and the empty delivery is a real one: whatever Rust reports still
  // missing after it is fetched in the next round, exactly as for a hull that
  // did come off the wire. Without this a pack hull's include fragments would
  // never be asked for.
  it('follows the include closure of a hull the overlay supplied', async () => {
    const f = fetcherOver({ 'frag/a': 'FA' });
    const p = pusherOver({ 'pack-hull': ['frag/a'] });

    const report = await deliverCatalogTemplates({
      paths: ['pack-hull'],
      fetchText: f.fetchText,
      pushTemplate: p.pushTemplate,
    });

    expect(report.unfetched).toEqual(['pack-hull']);
    expect(report.delivered).toEqual(['frag/a']);
    expect(p.pushed.map((x) => x.isRoot)).toEqual([true, false]);
  });

  // A fragment that is genuinely absent is offered to the overlay too, and its
  // root is then simply never resolved. What must NOT happen is the walk
  // spinning: `attempted` still closes the loop.
  it('asks for an absent fragment once and then stops', async () => {
    const f = fetcherOver({ hull: 'H' });
    const p = pusherOver({ hull: ['frag/gone'] });

    const report = await deliverCatalogTemplates({
      paths: ['hull'],
      fetchText: f.fetchText,
      pushTemplate: p.pushTemplate,
    });

    expect(f.requested.filter((x) => x === 'frag/gone')).toEqual(['frag/gone']);
    expect(report.unfetched).toEqual(['frag/gone']);
    expect(report.truncated).toBe(false);
  });

  it('drops the one hull the push refuses, and never rejects', async () => {
    const f = fetcherOver({ good: 'G', bad: 'B' });

    const report = await deliverCatalogTemplates({
      paths: ['good', 'bad'],
      fetchText: f.fetchText,
      pushTemplate: (path) => {
        if (path === 'bad') throw new Error('parse error');
        return [];
      },
    });

    expect(report.delivered).toEqual(['good']);
    expect(report.unfetched).toEqual([]);
    expect(report.failed).toEqual(['bad']);
  });

  it('stops at the round cap and says so instead of walking for ever', async () => {
    // Every delivery reveals one more fragment: an unbounded legal chain.
    let n = 0;
    const report = await deliverCatalogTemplates({
      paths: ['hull'],
      fetchText: async () => 'toml',
      pushTemplate: () => {
        n += 1;
        return [`frag/${n}`];
      },
      maxRounds: 3,
    });

    expect(report.rounds).toBe(3);
    expect(report.truncated).toBe(true);
    expect(report.delivered).toHaveLength(3);
  });

  it('does nothing at all for an empty path list', async () => {
    const report = await deliverCatalogTemplates({
      paths: [],
      fetchText: () => { throw new Error('should not fetch'); },
      pushTemplate: () => { throw new Error('should not push'); },
    });
    expect(report).toEqual({
      delivered: [], unfetched: [], failed: [], rounds: 0, truncated: false,
    });
  });

  it('exports a round cap deep enough for a shipped hull and its fragments', () => {
    expect(MAX_INCLUDE_ROUNDS).toBeGreaterThanOrEqual(2);
  });
});
