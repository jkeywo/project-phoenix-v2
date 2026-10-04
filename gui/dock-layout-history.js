import { createDockLayoutModel } from './dock-layout-model.js';
import { createDockLayoutMigration } from './dock-layout-migration.js';

/** Build version vocabularies and placement passes from ordered panel records.
 * `retired` is the first version that no longer accepts the panel. Registry
 * order follows records; migration order follows `migration.order`. Defaults
 * and per-generation policies belong to the context, including alias versions.
 * Omitting `migration` registers a panel without opening it during migration. */
export function createDockLayoutHistory({ version, panels, aliases = {}, defaultLayout, policy = () => ({}), rehome }) {
  const recordsFor = generation => panels.filter(panel => panel.since <= generation
    && (panel.retired === undefined || generation < panel.retired));
  const registryFor = generation => Object.freeze(recordsFor(generation)
    .map(({ id, kind }) => Object.freeze({ id, kind })));
  const registry = registryFor(version);
  const models = new Map();
  const modelFor = generation => {
    const canonical = aliases[generation] ?? generation;
    if (!models.has(canonical)) {
      models.set(canonical, createDockLayoutModel({ ...policy(canonical), version: canonical,
        panels: canonical === version ? registry : registryFor(canonical),
        defaultLayout: () => defaultLayout(canonical),
        compatibleVersions: Array.from({ length: version }, (_, i) => i + 1)
          .filter(candidate => (aliases[candidate] ?? candidate) === canonical) }));
    }
    return models.get(canonical);
  };
  const current = modelFor(version);
  const generations = Array.from({ length: version }, (_, index) => {
    const generation = index + 1;
    return { version: generation, model: modelFor(generation),
      added: panels.filter(panel => panel.since === generation && panel.migration)
        .sort((a, b) => (a.migration.order ?? 0) - (b.migration.order ?? 0))
        .map(({ id, migration }) => [id, migration.target, migration.placement ?? 'tab']) };
  });
  return Object.freeze({ registry, current,
    migrate: createDockLayoutMigration({ version, current, generations, rehome }) });
}
