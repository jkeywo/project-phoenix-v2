/** Closed generic creation inputs and finite own-System protection composition. */
const positive = value => typeof value === 'number' && Number.isFinite(Math.fround(value)) && Math.fround(value) > 0;
const uint = value => Number.isInteger(value) && value > 0 && value <= 0xffffffff;
const identity = value => typeof value === 'string' && value.trim() !== '' && !/[\x00-\x1f\x7f]/u.test(value);
export function validSystemPriorities(values) {
  return Array.isArray(values) && values.length <= 64 && new Set(values).size === values.length && values.every(identity);
}
export function validateChannelInitials(channels) {
  for (const channel of channels) {
    if (!['boolean', 'entity_target', 'system_priority'].includes(channel.kind)) throw new Error('Unsupported command channel kind.');
    const initial = channel.initial;
    if (initial == null) continue;
    if (typeof initial !== 'object' || Array.isArray(initial) || initial.kind !== channel.kind
        || Object.keys(initial).some(key => !['kind', 'value'].includes(key))) throw new Error('Initial value must match its exact command channel kind.');
    if (channel.kind === 'boolean' && typeof initial.value !== 'boolean'
        || channel.kind === 'system_priority' && !validSystemPriorities(initial.value)
        || channel.kind === 'entity_target' && initial.value != null
          && (!identity(initial.value) || new TextEncoder().encode(initial.value).length > 128)) throw new Error('Invalid bounded initial command value.');
  }
}
export const initialProtection = () => ({
  components: [{kind: 'power_input', id: 'input'}, {kind: 'signal_protection', id: 'protection', power_input: 'input',
    enabled_channel: 'enabled', priorities_channel: 'priorities', config: {capacity: 3, strength: 4, systems: {}}}],
  channels: [{name: 'enabled', kind: 'boolean', initial: {kind: 'boolean', value: true}},
    {name: 'priorities', kind: 'system_priority', initial: {kind: 'system_priority', value: []}}],
});
export function validateProtectionComposition(system, components, channels, entity = null) {
  if (system.kind !== 'signal-protection') return;
  validateChannelInitials(channels);
  const protection = components.find(row => row.kind === 'signal_protection'), config = protection?.config;
  const ids = components.map(row => row.id);
  const costs = Object.entries(config?.systems || {});
  if (!system.power_group || components.length !== 2 || new Set(ids).size !== ids.length
      || !ids.every(value => typeof value === 'string' && /^[a-z0-9-]+$/.test(value))
      || !protection || Object.keys(protection).some(key => !['id', 'kind', 'power_input', 'enabled_channel', 'priorities_channel', 'config'].includes(key))
      || components.filter(row => row.kind === 'power_input' && row.id === protection.power_input).length !== 1
      || Object.keys(config || {}).some(key => !['capacity', 'strength', 'systems'].includes(key))
      || !config?.systems || typeof config.systems !== 'object' || Array.isArray(config.systems)
      || !uint(config?.capacity) || !positive(config?.strength) || !costs.length || costs.length > 64
      || costs.some(([id, cost]) => !identity(id) || !uint(cost) || cost > config.capacity)
      || channels.length !== 2 || new Set(channels.map(row => row.name)).size !== 2
      || !channels.every(row => typeof row.name === 'string' && /^[a-z0-9-]+$/.test(row.name))
      || !channels.some(row => row.name === protection.enabled_channel && row.kind === 'boolean')
      || !channels.some(row => row.name === protection.priorities_channel && row.kind === 'system_priority')) {
    throw new Error('Use one actual power input, enabled/priorities channels and finite compatible System costs.');
  }
  const priorities = channels.find(row => row.name === protection.priorities_channel).initial?.value || [];
  if (priorities.some(id => !Object.hasOwn(config.systems, id))) throw new Error('Initial priority names an ineligible System.');
  if (entity) {
    const eligible = new Set([entity.comms?.radio?.system,
      ...['broad', 'focused', 'passive'].flatMap(kind => (entity.sensing?.[kind] || []).map(row => row.system))].filter(Boolean));
    const actual = new Set((entity.system || []).map(row => row.id));
    if (costs.some(([id]) => !actual.has(id) || !eligible.has(id))) throw new Error('Protection requires actual compatible own receiver equipment.');
  }
}
