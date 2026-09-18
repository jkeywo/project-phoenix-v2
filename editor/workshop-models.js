/** Model authoring over the Workshop's source owner. No filesystem, live ECS,
 * model serializer or separate history. Runtime fields retain exact TOML text. */
export function modelDocuments(paths) {
  const models = new Map();
  for (const path of paths) {
    if (!path.startsWith('assets/models/')) continue;
    const glb = /^(.*)\.glb$/.exec(path);
    const rig = /^(.*)\.([A-Za-z0-9_-]+)\.toml$/.exec(path);
    const stem = glb?.[1] || rig?.[1];
    if (!stem) continue;
    if (!models.has(stem)) models.set(stem, { model: `${stem}.glb`, variants: [] });
    if (rig) models.get(stem).variants.push({ name: rig[2], path });
  }
  return [...models.values()].sort((a, b) => a.model.localeCompare(b.model))
    .map(entry => ({ ...entry, variants: entry.variants.sort((a, b) =>
      a.name === 'model' ? -1 : b.name === 'model' ? 1 : a.name.localeCompare(b.name)) }));
}

/** Entity templates the preview can show as a composed subject.
 *
 * The shared viewer dispatches an entity's `[star]`, `[planet]` or `[mesh]`
 * visual through the same constructors the game uses, so a star and a planet
 * are previewable exactly by being entity templates — there is no separate
 * star list and no separate planet list to keep in step with the authored
 * content. Rig sidecars under assets/models/ are not templates and are
 * excluded; those are reached through their model instead.
 */
export function entityDocuments(paths) {
  return paths
    .filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml'))
    .sort((a, b) => a.localeCompare(b));
}

export function modelFieldGroup(field) {
  return ['base', 'markers', 'target_points', 'lod', 'extents', 'base_build'].includes(field.path[0])
    ? field.path[0] : 'other';
}

/** Clone the exact authored source, including its comments and line endings.
 * An absent sidecar starts with the runtime's empty identity rig. */
export function createModelVariant(draft, model, name, sourcePath = null) {
  if (!modelDocuments(draft.paths()).some(entry => entry.model === model)
    || !/^[A-Za-z0-9_-]{1,64}$/.test(name)) throw new Error('workshop.models.invalid_variant');
  const path = `${model.slice(0, -4)}.${name}.toml`;
  if (draft.paths().includes(path)) throw new Error('workshop.models.variant_exists');
  const variants = modelDocuments(draft.paths()).find(entry => entry.model === model).variants;
  if (sourcePath && !variants.some(entry => entry.path === sourcePath)) throw new Error('workshop.inspector_stale');
  const source = sourcePath ? draft.read(sourcePath) : '';
  if (typeof source !== 'string') throw new Error('workshop.inspector_stale');
  draft.put(path, source);
  return path;
}

/** Prepare all changed scalar spans through the same runtime inspector used
 * by the generic source form, then make ONE edit in chronological history.
 * The draft remains untouched on any refusal, stale result or context change. */
export async function patchModelFields({ draft, runtime, documentPath, source, fields, values,
  current = () => true }) {
  const unchanged = () => current() && draft.read(documentPath) === source;
  if (!unchanged() || fields.length !== values.length) throw new Error('workshop.inspector_stale');
  let result = source;
  for (let index = 0; index < fields.length; index++) {
    const value = values[index];
    if (typeof value !== 'string') throw new Error('workshop.inspector_refused');
    if (value === fields[index].source) continue;
    result = await runtime.patch(result, { document_path: documentPath, path: fields[index].path,
      expected_source: result, value_source: value });
    if (typeof result !== 'string') throw new Error('workshop.inspector_refused');
    if (!unchanged()) throw new Error('workshop.inspector_stale');
  }
  if (!unchanged()) throw new Error('workshop.inspector_stale');
  return draft.edit(documentPath, result);
}
