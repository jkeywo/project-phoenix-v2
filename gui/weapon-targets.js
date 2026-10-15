// Wire views distinguish a withheld contact from explicit Hold. Legacy fixtures
// with string/null group targets are normalized without granting actuation.
const reference = value => value?.mode === 'target' && typeof value.contact === 'string' && value.contact
  ? { mode: 'target', contact: value.contact } : value?.mode === 'hold' ? { mode: 'hold' } : { mode: 'unavailable' };
function assignmentView(value) {
  const group = value?.group;
  const target = group && (group.target == null || typeof group.target === 'string'
    ? group.target == null ? { mode: 'hold' } : { mode: 'target', contact: group.target } : reference(group.target));
  return { automatic: value?.automatic == null ? null : reference(value.automatic),
    individual: value?.individual == null ? null : reference(value.individual),
    group: group ? { group: group.group, target: target.mode === 'target' ? target.contact : null,
      unavailable: target.mode === 'unavailable' } : null };
}
/** Projection only: assignment intent never adds observation or authority. */
export const WEAPON_KINDS = Object.freeze(['phaser_bank', 'blaster_bank', 'torpedo_tube']);
export function weaponTargetsPayload(state, owned, assignments, blips) {
  const owns = new Set(owned), kinds = state.systemKinds || {};
  const rows = [...new Set([...owned, ...Object.keys(assignments || {})])].filter(id => WEAPON_KINDS.includes(kinds[id])).sort().map(system => ({
    system, label: state.systemStatus?.[system]?.display_name || system, assignment: assignmentView(assignments?.[system]),
    auto: state.controlSources?.[system] === 'Ai',
    controllable: owns.has(system) && state.controlSources?.[system] === 'Human',
  }));
  return {
    rows,
    groups: structuredClone(state.weaponGroups || []).filter(group => group.members?.some(id => rows.some(row => row.system === id))),
    contacts: (blips || []).filter(row => typeof row.uuid === 'string' && row.uuid && row.selectable === true && !row.predicted && row.kind !== 'waypoint')
      .map(row => ({ id: row.uuid, name: row.name || null })),
  };
}
