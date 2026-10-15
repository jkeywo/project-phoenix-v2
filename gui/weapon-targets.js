/** Projection only: assignment intent never adds observation or authority. */
export const WEAPON_KINDS = Object.freeze(['phaser_bank', 'blaster_bank', 'torpedo_tube']);
export function weaponTargetsPayload(state, owned, assignments, blips) {
  const owns = new Set(owned), kinds = state.systemKinds || {};
  const rows = [...new Set([...owned, ...Object.keys(assignments || {})])].filter(id => WEAPON_KINDS.includes(kinds[id])).sort().map(system => ({
    system, label: state.systemStatus?.[system]?.display_name || system, assignment: structuredClone(assignments?.[system] || { group: null, individual: null }),
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
