/** Physical generated equipment in the authoring view; execution remains Rust-owned. */
export const shieldArcSystemId = id => id ? `shield-arc-${id.replaceAll('_', '-')}` : null;
export function authoredAndGeneratedSystems(value) {
  const systems = [...(value.system || [])];
  const shields = systems.find(system => system.kind === 'shields');
  for (const arc of value.shield_arc || []) {
    const id = shieldArcSystemId(arc.id);
    if (!id) continue;
    if (!systems.some(system => system.id === id)) systems.push({ id, kind: 'shield_arc', console: shields?.console || 'generated-shields',
      shield_arc_id: arc.id, shield_energy: structuredClone(arc.energy), config: { ai: structuredClone(arc.ai) },
      ...(value.energy?.consumer?.some(row => row.system === id) ? { power_group: `energy-${id}` } : {}), generated: true });
  }
  return systems;
}
