import { describe, it, expect } from 'vitest';
import { CrossReferenceIndex } from '../cross-references.js';
import { getWorldContentData } from '../world-content-panel.js';

describe('getWorldContentData', () => {
  it('returns 5 lists (anchors, entities, triggers, comms, objectives)', () => {
    const crossRefIndex = new CrossReferenceIndex();
    const worldState = {
      anchors: { origin: [0, 0, 0] },
      entity: [{ name: 'alpha' }],
      trigger: [{ condition: 'on_destroyed', entity: 'alpha' }],
      comms: [{ from: 'alpha', entity: 'alpha', message: 'hi' }],
    };
    crossRefIndex.indexLayers([{ path: 'test.toml', worldState }]);
    const data = getWorldContentData(worldState, crossRefIndex, 'test.toml');
    expect(data).toHaveProperty('anchors');
    expect(data).toHaveProperty('namedEntities');
    expect(data).toHaveProperty('triggers');
    expect(data).toHaveProperty('commsTemplates');
    expect(data).toHaveProperty('objectives');
    expect(Array.isArray(data.anchors)).toBe(true);
    expect(Array.isArray(data.namedEntities)).toBe(true);
    expect(Array.isArray(data.triggers)).toBe(true);
    expect(Array.isArray(data.commsTemplates)).toBe(true);
    expect(Array.isArray(data.objectives)).toBe(true);
  });

  it('objectives list derived from add_objective actions', () => {
    const crossRefIndex = new CrossReferenceIndex();
    const worldState = {
      anchors: { a: [0, 0, 0] },
      entity: [],
      trigger: [{
        condition: 'on_destroyed',
        entity: 'raider',
        action: [{ type: 'add_objective', id: 'obj-1', text: 'Do it', mandatory: true }],
      }],
      comms: [{
        from: 'station',
        entity: 'station',
        message: 'hello',
        response: [{
          text: 'ok',
          action: [{ type: 'add_objective', id: 'obj-2', text: 'Do that' }],
        }],
      }],
    };
    crossRefIndex.indexLayers([{ path: 'test.toml', worldState }]);
    const data = getWorldContentData(worldState, crossRefIndex, 'test.toml');
    expect(data.objectives).toHaveLength(2);
    const obj1 = data.objectives.find(o => o.id === 'obj-1');
    expect(obj1).toBeTruthy();
    expect(obj1.text).toBe('Do it');
    expect(obj1.mandatory).toBe(true);
    const obj2 = data.objectives.find(o => o.id === 'obj-2');
    expect(obj2).toBeTruthy();
    expect(obj2.text).toBe('Do that');
  });

  it('cross-reference counts included in returned data', () => {
    const crossRefIndex = new CrossReferenceIndex();
    const worldState = {
      anchors: { starbase_alpha: [500, 0, 0] },
      entity: [
        { name: 'Starbase Alpha', template_path: 'station.toml', transform: { anchor: 'starbase_alpha' } },
        { name: 'raider', template_path: 'raider.toml', transform: { anchor: 'starbase_alpha' } },
      ],
      trigger: [
        { condition: 'on_attacked', entity: 'Starbase Alpha', action: [] },
        { condition: 'on_destroyed', entity: 'raider', action: [{ type: 'add_objective', id: 'obj-1' }] },
      ],
      comms: [{ from: 'Starbase Alpha', entity: 'Starbase Alpha', message: 'hi' }],
    };
    crossRefIndex.indexLayers([{ path: 'test.toml', worldState }]);
    const data = getWorldContentData(worldState, crossRefIndex, 'test.toml');
    const anchor = data.anchors.find(a => a.name === 'starbase_alpha');
    expect(anchor.refCount).toBe(2);
    const entity = data.namedEntities.find(e => e.name === 'Starbase Alpha');
    expect(entity.refCount).toBeGreaterThanOrEqual(2);
  });
});
