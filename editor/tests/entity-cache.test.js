import { describe, it, expect, beforeEach } from 'vitest';
import {
  entityCache,
  loadEntityConfig,
  getEntityConfig,
  preloadEntityCache,
  preloadEntityList,
  invalidateEntity,
  invalidateAll,
  onInvalidate,
} from '../entity-cache.js';
import { _setRootHandleForTest } from '../project-root.js';

function makeFileHandle(content) {
  return {
    kind: 'file',
    getFile: async () => ({ text: async () => content }),
  };
}

function makeAssetsEntitiesRoot(files) {
  const entitiesDir = {
    kind: 'directory',
    entries: async function* () {
      for (const [name, content] of Object.entries(files)) {
        yield [name, makeFileHandle(content)];
      }
    },
    getFileHandle: async (name) => {
      if (!(name in files)) {
        throw new DOMException(`File "${name}" not found`, 'NotFoundError');
      }
      return makeFileHandle(files[name]);
    },
  };
  const assetsDir = {
    kind: 'directory',
    getDirectoryHandle: async (name) => {
      if (name === 'entities') return entitiesDir;
      throw new DOMException(`Directory "${name}" not found`, 'NotFoundError');
    },
  };
  return {
    kind: 'directory',
    getDirectoryHandle: async (name) => {
      if (name === 'assets') return assetsDir;
      throw new DOMException(`Directory "${name}" not found`, 'NotFoundError');
    },
  };
}

// Minimal global TOML parser stub for tests.
if (!globalThis.window) globalThis.window = globalThis;
globalThis.window.tomlParse = (text) => {
  // Very small parser: just extracts `tags = ["a","b"]` and `name = "..."` lines.
  const out = {};
  const tagsMatch = text.match(/tags\s*=\s*\[([^\]]*)\]/);
  if (tagsMatch) {
    out.tags = tagsMatch[1]
      .split(',')
      .map((s) => s.trim().replace(/^"|"$/g, ''))
      .filter(Boolean);
  }
  const nameMatch = text.match(/^name\s*=\s*"([^"]*)"/m);
  if (nameMatch) out.name = nameMatch[1];
  return out;
};

describe('entity-cache (FSA)', () => {
  beforeEach(() => {
    entityCache.clear();
    _setRootHandleForTest(null);
  });

  it('preloadEntityCache no-ops gracefully when no root is selected', async () => {
    const results = await preloadEntityCache();
    expect(results).toEqual([]);
    expect(entityCache.size).toBe(0);
  });

  it('preloadEntityCache loads every .toml from assets/entities', async () => {
    const root = makeAssetsEntitiesRoot({
      'ship_a.toml': 'name = "Ship A"\ntags = ["ship","alpha"]\n',
      'ship_b.toml': 'name = "Ship B"\ntags = ["ship","beta"]\n',
      'README.md': 'not toml',
    });
    _setRootHandleForTest(root);

    const results = await preloadEntityCache();
    expect(results).toHaveLength(2);
    expect(entityCache.size).toBe(2);
    expect(entityCache.has('assets/entities/ship_a.toml')).toBe(true);
    expect(entityCache.has('assets/entities/ship_b.toml')).toBe(true);
  });

  it('loadEntityConfig caches and returns parsed config', async () => {
    const root = makeAssetsEntitiesRoot({
      'ship_a.toml': 'name = "Ship A"\ntags = ["ship"]\n',
    });
    _setRootHandleForTest(root);

    const cfg = await loadEntityConfig('assets/entities/ship_a.toml');
    expect(cfg).not.toBeNull();
    expect(cfg.tags).toEqual(['ship']);
    expect(getEntityConfig('assets/entities/ship_a.toml')).toBe(cfg);
  });

  it('loadEntityConfig returns null on missing file', async () => {
    const root = makeAssetsEntitiesRoot({});
    _setRootHandleForTest(root);

    const cfg = await loadEntityConfig('assets/entities/missing.toml');
    expect(cfg).toBeNull();
  });

  it('preloadEntityList returns name/path/tags from cache', async () => {
    const root = makeAssetsEntitiesRoot({
      'ship_a.toml': 'tags = ["ship"]\n',
    });
    _setRootHandleForTest(root);
    await preloadEntityCache();

    const list = preloadEntityList();
    expect(list).toEqual([
      { name: 'ship_a', path: 'assets/entities/ship_a.toml', tags: ['ship'] },
    ]);
  });

  it('invalidateEntity removes the entry and fires listeners', async () => {
    const root = makeAssetsEntitiesRoot({
      'ship_a.toml': 'tags = ["ship"]\n',
    });
    _setRootHandleForTest(root);
    await loadEntityConfig('assets/entities/ship_a.toml');
    expect(entityCache.has('assets/entities/ship_a.toml')).toBe(true);

    const fired = [];
    const sub = onInvalidate((p) => fired.push(p));
    invalidateEntity('assets/entities/ship_a.toml');

    expect(entityCache.has('assets/entities/ship_a.toml')).toBe(false);
    expect(fired).toEqual(['assets/entities/ship_a.toml']);
    sub.unsubscribe();
  });

  it('invalidateAll clears the cache and fires listeners with null', async () => {
    const root = makeAssetsEntitiesRoot({
      'ship_a.toml': 'tags = ["ship"]\n',
      'ship_b.toml': 'tags = ["ship"]\n',
    });
    _setRootHandleForTest(root);
    await preloadEntityCache();
    expect(entityCache.size).toBe(2);

    const fired = [];
    const sub = onInvalidate((p) => fired.push(p));
    invalidateAll();

    expect(entityCache.size).toBe(0);
    expect(fired).toEqual([null]);
    sub.unsubscribe();
  });

  it('onInvalidate.unsubscribe stops further notifications', async () => {
    const fired = [];
    const sub = onInvalidate((p) => fired.push(p));
    sub.unsubscribe();
    invalidateEntity('any');
    expect(fired).toEqual([]);
  });
});
