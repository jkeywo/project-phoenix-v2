export function getWorldContentData(worldState, crossRefIndex, activeLayerPath) {
  const anchors = [];
  const namedEntities = [];
  const triggers = [];
  const commsTemplates = [];
  const objectives = [];

  if (!worldState || typeof worldState !== 'object') {
    return { anchors, namedEntities, triggers, commsTemplates, objectives };
  }

  if (worldState.anchors && typeof worldState.anchors === 'object') {
    for (const [name, position] of Object.entries(worldState.anchors)) {
      const refCount = Array.isArray(worldState.entity)
        ? worldState.entity.filter(e => e.anchor === name).length
        : 0;
      anchors.push({ name, position, refCount });
    }
  }

  if (Array.isArray(worldState.entity)) {
    for (const ent of worldState.entity) {
      if (ent.name) {
        const refs = crossRefIndex.findReferences(ent.name);
        const refCount = refs.filter(r => r.layerPath === activeLayerPath).length;
        namedEntities.push({
          name: ent.name,
          template_path: ent.template_path,
          refCount,
        });
      }
    }
  }

  if (Array.isArray(worldState.trigger)) {
    for (const trig of worldState.trigger) {
      triggers.push({
        condition: trig.condition,
        entity: trig.entity,
        actionCount: Array.isArray(trig.action) ? trig.action.length : 0,
        refCount: 0,
      });
    }
  }

  if (Array.isArray(worldState.comms)) {
    for (const comms of worldState.comms) {
      commsTemplates.push({
        from: comms.from,
        trigger: comms.trigger,
        entity: comms.entity,
        responseCount: Array.isArray(comms.response) ? comms.response.length : 0,
        refCount: 0,
      });
    }
  }

  if (Array.isArray(worldState.trigger)) {
    for (const trig of worldState.trigger) {
      if (Array.isArray(trig.action)) {
        for (const action of trig.action) {
          if (action.type === 'add_objective' && action.id) {
            const existing = objectives.find(o => o.id === action.id);
            if (existing) {
              existing.refCount++;
            } else {
              objectives.push({ id: action.id, text: action.text, mandatory: action.mandatory, refCount: 1 });
            }
          }
        }
      }
    }
  }

  if (Array.isArray(worldState.comms)) {
    for (const comms of worldState.comms) {
      if (Array.isArray(comms.response)) {
        for (const resp of comms.response) {
          const actions = Array.isArray(resp.action) ? resp.action : [];
          for (const action of actions) {
            if (action.type === 'add_objective' && action.id) {
              const existing = objectives.find(o => o.id === action.id);
              if (existing) {
                existing.refCount++;
              } else {
                objectives.push({ id: action.id, text: action.text, mandatory: action.mandatory, refCount: 1 });
              }
            }
          }
        }
      }
    }
  }

  return { anchors, namedEntities, triggers, commsTemplates, objectives };
}
