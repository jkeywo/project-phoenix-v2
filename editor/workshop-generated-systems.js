/** Physical generated equipment in the authoring view; execution remains Rust-owned. */
export function authoredAndGeneratedSystems(value) {
  const systems = [...(value.system || [])];
  const shields = systems.find(system => system.kind === 'shields');
  if (shields) for (const arc of value.shield_arc || []) {
    const id = `shield-arc-${arc.id}`;
    if (!systems.some(system => system.id === id)) systems.push({ id, kind: 'shield_arc', console: shields.console, generated: true });
  }
  return systems;
}
