/** Forward migration between registered dock-panel vocabularies.
 *
 * A stored layout is sanitized against the vocabulary its OWN version declared,
 * then the panels registered since are placed one at a time. A panel therefore
 * only ever enters an arrangement through this file, never by being read back
 * out of an older tree, and a panel the operator explicitly closed under the
 * stored version stays closed.
 *
 * Placement carries no authority: it decides where a newly registered panel
 * appears the first time an existing operator sees it, and nothing else. */

export function firstVisible(node, floats = []) {
  if (node?.type === 'tabs') return node.active;
  if (node?.type === 'split') return node.children.map(child => firstVisible(child)).find(Boolean);
  return floats[0]?.panel || null;
}

export function containsPanel(node, panel) {
  if (node?.type === 'tabs') return node.tabs.includes(panel);
  return node?.type === 'split' && node.children.some(child => containsPanel(child, panel));
}

/** Place one newly registered panel beside the group it belongs with.
 *
 * `preferred` is the panel whose group it joins. When that panel is not in the
 * tree — closed, floating, or itself newly registered and not yet placed — the
 * first visible panel stands in, so the new panel lands somewhere reachable
 * rather than nowhere. */
export function addMigrationPanel(state, panel, preferred, model, preservedClosed = [], placement = 'tab') {
  if (!state.closed.includes(panel) || preservedClosed.includes(panel)) return state;
  const target = containsPanel(state.root, preferred) ? preferred : firstVisible(state.root);
  if (target) return model.dock(state, panel, target, placement);
  return state.floats.length ? model.reopen(state, panel) : state;
}

/** Which tab each group was showing, so migration does not change the view the
 * operator left the surface on just because it appended tabs to that group. */
export function activePanels(node, out = new Map()) {
  if (node?.type === 'tabs') node.tabs.forEach(panel => out.set(panel, node.active));
  else if (node?.type === 'split') node.children.forEach(child => activePanels(child, out));
  return out;
}

/** A group made entirely of panels this migration introduced has no view the
 * operator ever chose, and placement leaves it showing whichever panel was
 * docked last. Show the first instead, so a brand-new group opens at its
 * beginning rather than at its end. */
function settleNewGroups(node, added) {
  if (node?.type === 'tabs') {
    if (node.tabs.every(panel => added.has(panel))) node.active = node.tabs[0];
  } else if (node?.type === 'split') node.children.forEach(child => settleNewGroups(child, added));
}

export function restoreActives(node, previous) {
  if (node?.type === 'tabs') {
    const active = node.tabs.map(panel => previous.get(panel)).find(panel => node.tabs.includes(panel));
    if (active) node.active = active;
  } else if (node?.type === 'split') node.children.forEach(child => restoreActives(child, previous));
}

/**
 * Build a `normalize` that migrates forward through a chain of vocabularies.
 *
 * `generations` is ordered oldest first, each `{ version, model, panels, added }`,
 * where `added` is the `[panel, preferred, placement?]` list that version
 * introduced and `model` sanitizes a tree stored at that version. The last
 * entry is the current registry. `rehome` runs an extra placement pass for a
 * specific stored version, for vocabularies that gained panels without a
 * version bump.
 */
export function createDockLayoutMigration({ version, generations, current, rehome }) {
  const stored = generations.map(generation => generation.version);
  return function migrate(value, bounds) {
    const from = value?.version;
    const index = stored.indexOf(from);
    if (index < 0 || index === generations.length - 1) return current.normalize(value, bounds);
    const generation = generations[index];
    const normalized = generation.model.normalize(value, bounds);
    const added = generations.slice(index + 1).flatMap(later => later.added);
    let migrated = { ...normalized, version, closed: [...normalized.closed, ...added.map(([panel]) => panel)] };
    const previousActives = activePanels(normalized.root);
    migrated = rehome?.(migrated, from, value, generation) ?? migrated;
    migrated = current.normalize(migrated, bounds);
    for (const [panel, preferred, placement = 'tab'] of added) {
      migrated = addMigrationPanel(migrated, panel, preferred, current, [], placement);
    }
    settleNewGroups(migrated.root, new Set(added.map(([panel]) => panel)));
    restoreActives(migrated.root, previousActives);
    migrated.selected = normalized.selected;
    return migrated;
  };
}
