/**
 * world-content-panel.js — Pure data model behind the "World Content" tree.
 *
 * Lists what a world still declares in TOML: its `[anchors]` and its named
 * `[[entity]]` spawns. The trigger / comms-template / objective sections it
 * also used to build are gone with the declarative scenario front-end
 * (issue #985) — a world's triggers, dialogue and objectives are authored in
 * its `[script]` Rhai body now, which this TOML walk cannot read.
 */
export function getWorldContentData(worldState) {
  const anchors = [];
  const namedEntities = [];

  if (!worldState || typeof worldState !== 'object') {
    return { anchors, namedEntities };
  }

  if (worldState.anchors && typeof worldState.anchors === 'object') {
    for (const [name, position] of Object.entries(worldState.anchors)) {
      const refCount = Array.isArray(worldState.entity)
        ? worldState.entity.filter(e => e.transform && e.transform.anchor === name).length
        : 0;
      anchors.push({ name, position, refCount });
    }
  }

  if (Array.isArray(worldState.entity)) {
    for (const ent of worldState.entity) {
      if (ent.name) {
        namedEntities.push({
          name: ent.name,
          template_path: ent.template_path,
        });
      }
    }
  }

  return { anchors, namedEntities };
}
