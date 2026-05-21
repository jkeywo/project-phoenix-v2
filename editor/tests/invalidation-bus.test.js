import { describe, it, expect } from 'vitest';
import { InvalidationBus } from '../invalidation-bus.js';

describe('InvalidationBus', () => {
  describe('fireEntitySaved', () => {
    it('triggers registered callbacks with correct path', () => {
      const bus = new InvalidationBus();
      const calls = [];
      bus.onEntitySaved((path) => calls.push(path));
      bus.fireEntitySaved('assets/entities/pirate_raider.toml');
      expect(calls).toEqual(['assets/entities/pirate_raider.toml']);
    });
  });

  describe('fireWorldSaved', () => {
    it('triggers registered callbacks with correct path', () => {
      const bus = new InvalidationBus();
      const calls = [];
      bus.onWorldSaved((path) => calls.push(path));
      bus.fireWorldSaved('assets/worlds/default.toml');
      expect(calls).toEqual(['assets/worlds/default.toml']);
    });
  });

  describe('unsubscribe', () => {
    it('stops receiving events after unsubscribe', () => {
      const bus = new InvalidationBus();
      const calls = [];
      const { unsubscribe } = bus.onEntitySaved((path) => calls.push(path));
      bus.fireEntitySaved('a.toml');
      unsubscribe();
      bus.fireEntitySaved('b.toml');
      expect(calls).toEqual(['a.toml']);
    });

    it('stops receiving world events after unsubscribe', () => {
      const bus = new InvalidationBus();
      const calls = [];
      const { unsubscribe } = bus.onWorldSaved((path) => calls.push(path));
      bus.fireWorldSaved('a.toml');
      unsubscribe();
      bus.fireWorldSaved('b.toml');
      expect(calls).toEqual(['a.toml']);
    });
  });

  describe('multiple listeners', () => {
    it('all registered listeners receive the same event', () => {
      const bus = new InvalidationBus();
      const calls1 = [];
      const calls2 = [];
      bus.onEntitySaved((path) => calls1.push(path));
      bus.onEntitySaved((path) => calls2.push(path));
      bus.fireEntitySaved('shared.toml');
      expect(calls1).toEqual(['shared.toml']);
      expect(calls2).toEqual(['shared.toml']);
    });

    it('all world listeners receive the same event', () => {
      const bus = new InvalidationBus();
      const calls1 = [];
      const calls2 = [];
      bus.onWorldSaved((path) => calls1.push(path));
      bus.onWorldSaved((path) => calls2.push(path));
      bus.fireWorldSaved('shared.toml');
      expect(calls1).toEqual(['shared.toml']);
      expect(calls2).toEqual(['shared.toml']);
    });
  });

  describe('no listeners', () => {
    it('fireEntitySaved with no listeners does not throw', () => {
      const bus = new InvalidationBus();
      expect(() => bus.fireEntitySaved('anything.toml')).not.toThrow();
    });

    it('fireWorldSaved with no listeners does not throw', () => {
      const bus = new InvalidationBus();
      expect(() => bus.fireWorldSaved('anything.toml')).not.toThrow();
    });
  });

  describe('fireFactionSaved', () => {
    it('triggers registered callbacks with correct path', () => {
      const bus = new InvalidationBus();
      const calls = [];
      bus.onFactionSaved((path) => calls.push(path));
      bus.fireFactionSaved('assets/factions/federation.toml');
      expect(calls).toEqual(['assets/factions/federation.toml']);
    });

    it('onFactionSaved returns unsubscribe handle that stops events', () => {
      const bus = new InvalidationBus();
      const calls = [];
      const { unsubscribe } = bus.onFactionSaved((p) => calls.push(p));
      bus.fireFactionSaved('a.toml');
      unsubscribe();
      bus.fireFactionSaved('b.toml');
      expect(calls).toEqual(['a.toml']);
    });
  });
});
