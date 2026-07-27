import { describe, it, expect, beforeEach } from 'vitest';
import { TriggerableWorlds } from '../triggerable-worlds.js';

describe('TriggerableWorlds', () => {
  let tw;

  beforeEach(() => {
    tw = new TriggerableWorlds();
  });

  // ── 1. No layers ─────────────────────────────────────────────────────────────

  it('scanLayers with no layers returns empty array', () => {
    const paths = tw.scanLayers([]);
    expect(paths).toEqual([]);
  });

  // ── 2. Layer with no triggers ────────────────────────────────────────────────

  it('scanLayers with a layer that has no triggers returns empty array', () => {
    const layers = [
      { worldState: { global: { seed: 1 } }, path: 'assets/worlds/default.toml' },
    ];
    const paths = tw.scanLayers(layers);
    expect(paths).toEqual([]);
  });

  // ── 3. Layer with a load_world action ────────────────────────────────────────

  it('scanLayers with a layer that has a load_world action returns its path', () => {
    const layers = [
      {
        worldState: {
          trigger: [
            {
              condition: 'on_destroyed',
              entity: 'raider_alpha',
              action: [
                { type: 'load_world', path: 'assets/worlds/patrol.toml' },
              ],
            },
          ],
        },
        path: 'assets/worlds/default.toml',
      },
    ];
    const paths = tw.scanLayers(layers);
    expect(paths).toEqual(['assets/worlds/patrol.toml']);
  });

  // ── 4. Deduplication ─────────────────────────────────────────────────────────

  it('scanLayers deduplicates the same path referenced by two triggers', () => {
    const layers = [
      {
        worldState: {
          trigger: [
            {
              condition: 'on_destroyed',
              entity: 'raider_alpha',
              action: [
                { type: 'load_world', path: 'assets/worlds/patrol.toml' },
              ],
            },
            {
              condition: 'on_destroyed',
              entity: 'raider_bravo',
              action: [
                { type: 'load_world', path: 'assets/worlds/patrol.toml' },
              ],
            },
          ],
        },
        path: 'assets/worlds/default.toml',
      },
    ];
    const paths = tw.scanLayers(layers);
    expect(paths).toHaveLength(1);
    expect(paths).toContain('assets/worlds/patrol.toml');
  });

  // ── 5. Non-load_world actions ────────────────────────────────────────────────

  it('scanLayers ignores non-load_world trigger actions', () => {
    const layers = [
      {
        worldState: {
          trigger: [
            {
              condition: 'on_destroyed',
              entity: 'raider_alpha',
              action: [
                { type: 'spawn', template: 'assets/entities/ship_harrow_patrol.toml' },
                { type: 'message', text: 'Mayday!' },
              ],
            },
          ],
        },
        path: 'assets/worlds/default.toml',
      },
    ];
    const paths = tw.scanLayers(layers);
    expect(paths).toEqual([]);
  });

  // ── 6. Partial trigger objects ───────────────────────────────────────────────

  it('scanLayers handles partial trigger objects (no action array)', () => {
    const layers = [
      {
        worldState: {
          trigger: [
            { condition: 'on_destroyed', entity: 'raider_alpha' },
          ],
        },
        path: 'assets/worlds/default.toml',
      },
    ];
    // Should not throw
    const paths = tw.scanLayers(layers);
    expect(paths).toEqual([]);
  });

  it('scanLayers handles partial trigger objects (empty action array)', () => {
    const layers = [
      {
        worldState: {
          trigger: [
            { condition: 'on_destroyed', entity: 'raider_alpha', action: [] },
          ],
        },
        path: 'assets/worlds/default.toml',
      },
    ];
    const paths = tw.scanLayers(layers);
    expect(paths).toEqual([]);
  });

  it('scanLayers handles triggers where action is not an array', () => {
    const layers = [
      {
        worldState: {
          trigger: [
            { condition: 'on_timer', action: 'not-an-array' },
          ],
        },
        path: 'assets/worlds/default.toml',
      },
    ];
    const paths = tw.scanLayers(layers);
    expect(paths).toEqual([]);
  });

  it('scanLayers handles trigger being present but null', () => {
    const layers = [
      { worldState: { trigger: null }, path: 'assets/worlds/default.toml' },
    ];
    const paths = tw.scanLayers(layers);
    expect(paths).toEqual([]);
  });

  // ── 7. Toggle / isLoaded ─────────────────────────────────────────────────────

  it('togglePath toggles state; isLoaded returns correct state', () => {
    const path = 'assets/worlds/patrol.toml';

    expect(tw.isLoaded(path)).toBe(false);

    const state1 = tw.togglePath(path);
    expect(state1).toBe(true);
    expect(tw.isLoaded(path)).toBe(true);

    const state2 = tw.togglePath(path);
    expect(state2).toBe(false);
    expect(tw.isLoaded(path)).toBe(false);
  });

  it('togglePath toggles independently for different paths', () => {
    tw.togglePath('assets/worlds/a.toml');
    tw.togglePath('assets/worlds/b.toml');

    expect(tw.isLoaded('assets/worlds/a.toml')).toBe(true);
    expect(tw.isLoaded('assets/worlds/b.toml')).toBe(true);

    tw.togglePath('assets/worlds/a.toml');
    expect(tw.isLoaded('assets/worlds/a.toml')).toBe(false);
    expect(tw.isLoaded('assets/worlds/b.toml')).toBe(true);
  });

  // ── 8. Reset ─────────────────────────────────────────────────────────────────

  it('reset clears all toggles', () => {
    tw.togglePath('assets/worlds/patrol.toml');
    tw.togglePath('assets/worlds/second.toml');
    expect(tw.isLoaded('assets/worlds/patrol.toml')).toBe(true);
    expect(tw.isLoaded('assets/worlds/second.toml')).toBe(true);

    tw.reset();

    expect(tw.isLoaded('assets/worlds/patrol.toml')).toBe(false);
    expect(tw.isLoaded('assets/worlds/second.toml')).toBe(false);
  });

  it('reset does not affect paths list from scan', () => {
    const layers = [
      {
        worldState: {
          trigger: [
            { condition: 'on_destroyed', entity: 'x', action: [{ type: 'load_world', path: 'assets/worlds/patrol.toml' }] },
          ],
        },
        path: 'assets/worlds/default.toml',
      },
    ];
    tw.scanLayers(layers);
    expect(tw.getPaths()).toHaveLength(1);

    tw.togglePath('assets/worlds/patrol.toml');
    tw.reset();

    // getPaths is preserved (reset only clears toggles)
    expect(tw.getPaths()).toHaveLength(1);
    expect(tw.getPaths()).toContain('assets/worlds/patrol.toml');
  });

  // ── getPaths ─────────────────────────────────────────────────────────────────

  it('getPaths returns empty array when no scan has been done', () => {
    expect(tw.getPaths()).toEqual([]);
  });

  it('getPaths returns results from last scanLayers call', () => {
    const layers = [
      {
        worldState: {
          trigger: [
            { condition: 'on_destroyed', entity: 'x', action: [{ type: 'load_world', path: 'path_a.toml' }] },
            { condition: 'on_destroyed', entity: 'y', action: [{ type: 'load_world', path: 'path_b.toml' }] },
          ],
        },
        path: 'assets/worlds/default.toml',
      },
    ];
    tw.scanLayers(layers);
    expect(tw.getPaths()).toEqual(['path_a.toml', 'path_b.toml']);
  });
});
