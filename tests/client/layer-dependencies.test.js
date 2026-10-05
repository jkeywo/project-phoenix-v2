import { describe, it, expect } from 'vitest';
import { dependencyEntries, layerDependencyFailures } from '../../scripts/layer-policy.mjs';
const workspace = { dependencies: { session_backend: { package: 'phoenix-sim-session', path: 'crates/phoenix-sim-session' } } };
describe('simulation branch dependency rules', () => {
  for (const kind of ['dependencies', 'dev-dependencies', 'build-dependencies']) {
    for (const target of [null, 'cfg(target_arch = "wasm32")']) {
      it(`rejects an inherited sibling alias in ${kind} / ${target ?? 'all targets'}`, () => {
        const section = { [kind]: { session_backend: { workspace: true } } };
        const manifest = target ? { target: { [target]: section } } : section;
        expect(layerDependencyFailures('phoenix-sim-gameplay', manifest, workspace)).toEqual([
          `phoenix-sim-gameplay may not depend on phoenix-sim-session (${kind}, ${target ?? 'all targets'})`,
        ]);
        expect(dependencyEntries(manifest, workspace)[0].name).toBe('phoenix-sim-session');
      });
    }
  }
  it('allows common contracts and lets the parent compose the branches', () => {
    expect(layerDependencyFailures('phoenix-sim-world', { dependencies: { 'phoenix-sim-contracts': '*' } }, {})).toEqual([]);
    expect(layerDependencyFailures('phoenix-simulation', { dependencies: { session_backend: { workspace: true } } }, workspace)).toEqual([]);
  });
  it('rejects a branch importing its parent under a local alias', () => {
    expect(layerDependencyFailures('phoenix-sim-world', { dependencies: { adapter: { package: 'phoenix-simulation' } } }, {})).toHaveLength(1);
  });
  it('keeps inherited features visible to renderer checks', () => {
    const entries = dependencyEntries({ dependencies: { physics: { workspace: true, features: ['dim3'], 'default-features': false } } }, {
      dependencies: { physics: { package: 'bevy_rapier3d', features: ['async-collider'] } },
    });
    expect(entries[0].definition.features).toEqual(['async-collider', 'dim3']);
    expect(entries[0].definition['default-features']).toBe(true);
  });
});
