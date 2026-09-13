export const npcDescriptor = (kind, live_mutability, schema_path) => ({
  kind, default_source: null, live_mutability,
  origin: { schema_path, document: null, line: null, layer: null }, validation: [],
});
export const npcInspector = {
  current: npcDescriptor('enum', 'named-action', 'behaviour.doctrine'),
  intent: npcDescriptor('string', 'derived', 'scored_objectives.chosen'),
  definition: npcDescriptor('string', 'recreate-required', 'gm_npc_doctrine_palette.doctrine'),
};
export const npcProfile = { current: null, intent: null, inspector: npcInspector,
  choices: [{ id: 'north', label: 'North route', revision: '0123456789abcdef', origin_layer: null },
    { id: 'east', label: 'East route', revision: 'fedcba9876543210', origin_layer: 'supporting-world' }] };
