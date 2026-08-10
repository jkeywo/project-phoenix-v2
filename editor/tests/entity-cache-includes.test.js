import { describe, it, expect, beforeEach } from 'vitest';
import { parse as tomlParse } from 'smol-toml';
import {
  entityCache,
  loadEntityConfig,
  getEntityConfig,
  resolveEntityConfig,
  resolveEntityConfigFromText,
  getEntityResolution,
  invalidateAll,
} from '../entity-cache.js';
import { _setRootHandleForTest } from '../project-root.js';
import { sectionOrigin } from '../entity-includes.js';

// Issue #910 — the entity cache resolves a composed hull's include closure
// before handing panes its document, and surfaces a broken include as a located
// error instead of a silent null.

function makeFileHandle(content) {
  return { kind: 'file', getFile: async () => ({ text: async () => content }) };
}

/** A project root exposing assets/entities/<name> for the given files map. */
function makeEntitiesRoot(files) {
  const entitiesDir = {
    kind: 'directory',
    entries: async function* () {
      for (const [name, content] of Object.entries(files)) yield [name, makeFileHandle(content)];
    },
    getFileHandle: async (name) => {
      if (!(name in files)) throw new DOMException(`File "${name}" not found`, 'NotFoundError');
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

if (!globalThis.window) globalThis.window = globalThis;
globalThis.window.tomlParse = (text) => tomlParse(text);

describe('entity-cache include resolution (issue #910)', () => {
  beforeEach(() => {
    invalidateAll();
    _setRootHandleForTest(null);
  });

  it('resolves a composed hull so its systems come from the fragment', async () => {
    _setRootHandleForTest(
      makeEntitiesRoot({
        'systems.toml':
          'tags = ["ship"]\n[[system]]\nid = "helm-thrust"\nkind = "helm_thrust"\n[[system]]\nid = "power-reactor"\nkind = "power_reactor"\n',
        'hull.toml': 'includes = ["systems.toml"]\n[hull]\nhull_integrity = 500.0\n',
      }),
    );

    const config = await loadEntityConfig('assets/entities/hull.toml');
    expect(config).not.toBeNull();
    // The resolved document a validation/preview pane reads has the fragment's
    // systems — it does NOT appear to have none.
    expect(config.system.map((s) => s.id)).toEqual(['helm-thrust', 'power-reactor']);
    expect(config.hull.hull_integrity).toBe(500);
    expect(config.tags).toEqual(['ship']);

    // getEntityConfig returns the same resolved document.
    expect(getEntityConfig('assets/entities/hull.toml')).toBe(config);

    // The resolution record carries provenance the UI can badge from.
    const res = getEntityResolution('assets/entities/hull.toml');
    expect(res.ok).toBe(true);
    expect(res.isComposed).toBe(true);
    expect(res.sources).toContain('assets/entities/systems.toml');
    expect(sectionOrigin(res.provenance, 'system', 'assets/entities/hull.toml')).toBe('inherited');
    expect(sectionOrigin(res.provenance, 'hull', 'assets/entities/hull.toml')).toBe('authored');
  });

  it('surfaces a missing fragment as a located error naming the declaring hull', async () => {
    _setRootHandleForTest(
      makeEntitiesRoot({
        'hull.toml': 'includes = ["nope.toml"]\ntags = ["ship"]\n',
      }),
    );

    const config = await loadEntityConfig('assets/entities/hull.toml');
    expect(config).toBeNull(); // not a silent omission — the error is retrievable

    const res = getEntityResolution('assets/entities/hull.toml');
    expect(res.ok).toBe(false);
    expect(res.error.category).toBe('include-missing');
    expect(res.error.file).toBe('assets/entities/hull.toml');
    expect(res.error.chain).toContain('assets/entities/nope.toml');
  });

  it('surfaces an include cycle as a located error', async () => {
    _setRootHandleForTest(
      makeEntitiesRoot({
        'a.toml': 'includes = ["b.toml"]\n',
        'b.toml': 'includes = ["a.toml"]\n',
      }),
    );

    const res = await resolveEntityConfig('assets/entities/a.toml');
    expect(res.ok).toBe(false);
    expect(res.error.category).toBe('include-cycle');
  });

  it('resolveEntityConfigFromText composes LIVE root text against on-disk fragments', async () => {
    _setRootHandleForTest(
      makeEntitiesRoot({
        'systems.toml': 'tags = ["ship"]\n[[system]]\nid = "helm"\nkind = "helm_thrust"\n',
        // The on-disk hull authors hull_integrity = 100; the live edit below
        // sets 999. The save gate must resolve against the LIVE text.
        'hull.toml': 'includes = ["systems.toml"]\n[hull]\nhull_integrity = 100.0\n',
      }),
    );

    const liveText = 'includes = ["systems.toml"]\n[hull]\nhull_integrity = 999.0\n';
    const res = await resolveEntityConfigFromText('assets/entities/hull.toml', liveText);

    expect(res.ok).toBe(true);
    expect(res.isComposed).toBe(true);
    // Fragment field is merged in (systems come from the fragment)...
    expect(res.value.system.map((s) => s.id)).toEqual(['helm']);
    // ...and the LIVE hull value wins over the stale on-disk 100.
    expect(res.value.hull.hull_integrity).toBe(999);
  });

  it('resolveEntityConfigFromText surfaces a missing fragment as a located error', async () => {
    _setRootHandleForTest(makeEntitiesRoot({}));

    const res = await resolveEntityConfigFromText(
      'assets/entities/hull.toml',
      'includes = ["nope.toml"]\ntags = ["ship"]\n',
    );

    expect(res.ok).toBe(false);
    expect(res.error.category).toBe('include-missing');
    expect(res.error.file).toBe('assets/entities/hull.toml');
    expect(res.error.chain).toContain('assets/entities/nope.toml');
  });

  it('leaves an uncomposed entity verbatim and uncomposed', async () => {
    _setRootHandleForTest(
      makeEntitiesRoot({ 'plain.toml': 'tags = ["asteroid"]\n[collider]\nradius = 5.0\n' }),
    );

    const config = await loadEntityConfig('assets/entities/plain.toml');
    expect(config.tags).toEqual(['asteroid']);
    expect(config.collider.radius).toBe(5);
    const res = getEntityResolution('assets/entities/plain.toml');
    expect(res.isComposed).toBe(false);
    expect(entityCache.get('assets/entities/plain.toml')).toBe(config);
  });
});
