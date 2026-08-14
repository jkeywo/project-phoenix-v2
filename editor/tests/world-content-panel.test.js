import { describe, it, expect } from 'vitest';
import { getWorldContentData } from '../world-content-panel.js';

describe('getWorldContentData', () => {
  it('returns the two lists a scripted world still declares in TOML', () => {
    const worldState = {
      anchors: { origin: [0, 0, 0] },
      entity: [{ name: 'alpha' }],
      script: { setup: 'fn on_hailed() {}' },
    };
    const data = getWorldContentData(worldState);
    expect(Object.keys(data).sort()).toEqual(['anchors', 'namedEntities']);
    expect(Array.isArray(data.anchors)).toBe(true);
    expect(Array.isArray(data.namedEntities)).toBe(true);
  });

  it('returns empty lists for a missing or non-object world', () => {
    for (const bad of [null, undefined, 'nope']) {
      const data = getWorldContentData(bad);
      expect(data.anchors).toEqual([]);
      expect(data.namedEntities).toEqual([]);
    }
  });

  it('anchor refCount counts the spawns anchored to it', () => {
    const worldState = {
      anchors: { starbase_alpha: [500, 0, 0], unused: [0, 0, 0] },
      entity: [
        { name: 'Starbase Alpha', template_path: 'station.toml', transform: { anchor: 'starbase_alpha' } },
        { name: 'raider', template_path: 'raider.toml', transform: { anchor: 'starbase_alpha' } },
      ],
    };
    const data = getWorldContentData(worldState);
    expect(data.anchors.find(a => a.name === 'starbase_alpha').refCount).toBe(2);
    expect(data.anchors.find(a => a.name === 'unused').refCount).toBe(0);
  });

  it('lists only NAMED entities, carrying their template path', () => {
    const worldState = {
      anchors: {},
      entity: [
        { name: 'Starbase Alpha', template_path: 'station.toml' },
        { template_path: 'asteroid.toml' },  // anonymous — not listed
      ],
    };
    const data = getWorldContentData(worldState);
    expect(data.namedEntities).toEqual([
      { name: 'Starbase Alpha', template_path: 'station.toml' },
    ]);
  });
});
