// Cargo identity resolution shared by enforcement and the source-derived graph.
export const allowed = {
  'phoenix-math': [], 'phoenix-runtime': ['phoenix-math'],
  'phoenix-transport': ['phoenix-runtime'], 'phoenix-platform': [],
  'phoenix-model': ['phoenix-math', 'phoenix-transport'],
  'phoenix-content': ['phoenix-model', 'phoenix-math', 'phoenix-platform'],
  'phoenix-sim-contracts': ['phoenix-model', 'phoenix-content', 'phoenix-math', 'phoenix-runtime', 'phoenix-transport'],
  'phoenix-sim-gameplay': ['phoenix-sim-contracts', 'phoenix-model', 'phoenix-content', 'phoenix-math'],
  'phoenix-sim-world': ['phoenix-sim-contracts', 'phoenix-model', 'phoenix-content', 'phoenix-math'],
  'phoenix-sim-session': ['phoenix-sim-contracts', 'phoenix-model', 'phoenix-runtime', 'phoenix-transport'],
  'phoenix-simulation': ['phoenix-sim-contracts', 'phoenix-sim-gameplay', 'phoenix-sim-world', 'phoenix-sim-session', 'phoenix-runtime', 'phoenix-transport', 'phoenix-platform', 'phoenix-model', 'phoenix-content', 'phoenix-math'],
  'phoenix-presentation': ['phoenix-simulation', 'phoenix-model', 'phoenix-content', 'phoenix-math'],
  'phoenix-grid': ['phoenix-runtime', 'phoenix-transport', 'phoenix-platform', 'phoenix-math'],
};

export function resolveDependency(alias, declared, workspace = {}) {
  const local = typeof declared === 'string' ? {} : declared;
  if (!local.workspace) return local;
  const inherited = workspace.dependencies?.[alias];
  if (!inherited) throw new Error(`Missing workspace dependency ${alias}`);
  const base = typeof inherited === 'string' ? {} : inherited;
  return {
    ...base, ...local,
    // Cargo adds local features; it cannot remove inherited features/defaults.
    features: [...new Set([...(base.features || []), ...(local.features || [])])],
    'default-features': base['default-features'] === false ? (local['default-features'] ?? false) : true,
  };
}
export function dependencyEntries(manifest, workspace) {
  return [['all targets', manifest], ...Object.entries(manifest.target || {})].flatMap(([target, section]) =>
    ['dependencies', 'dev-dependencies', 'build-dependencies'].flatMap(kind =>
      Object.entries(section[kind] || {}).map(([alias, declared]) => {
        const definition = resolveDependency(alias, declared, workspace);
        return { alias, name: definition.package || alias, definition, target, kind };
      })));
}
export function layerDependencyFailures(name, manifest, workspace) {
  const permitted = allowed[name];
  if (!permitted) throw new Error(`Unclassified layer ${name}`);
  return dependencyEntries(manifest, workspace)
    .filter(entry => (entry.name.startsWith('phoenix-') || entry.name === 'project-phoenix') && !permitted.includes(entry.name))
    .map(entry => `${name} may not depend on ${entry.name} (${entry.kind}, ${entry.target})`);
}
