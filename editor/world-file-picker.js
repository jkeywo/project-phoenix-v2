export function getWorldTomlPaths(allFiles) {
  if (!Array.isArray(allFiles)) return [];
  return allFiles
    .filter((f) => f.startsWith('assets/worlds/') && f.endsWith('.toml'))
    .map((f) => ({ path: f, label: f.replace('assets/worlds/', '') }));
}

export function scanWorldActions(worldState) {
  const result = { hasLoadWorld: false, hasUnloadWorld: false, loadPaths: [], unloadPaths: [] };
  if (!worldState || !Array.isArray(worldState.trigger)) return result;
  for (const trigger of worldState.trigger) {
    if (!Array.isArray(trigger.action)) continue;
    for (const action of trigger.action) {
      if (!action || !action.type) continue;
      if (action.type === 'load_world' && action.path) {
        result.hasLoadWorld = true;
        result.loadPaths.push(action.path);
      }
      if (action.type === 'unload_world' && action.path) {
        result.hasUnloadWorld = true;
        result.unloadPaths.push(action.path);
      }
    }
  }
  return result;
}
