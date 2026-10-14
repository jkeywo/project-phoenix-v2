/** Generate supported authored Power nodes. Execution stays with ordinary System AI. */
export function configuredPowerTree(profile) {
  if (!profile?.reactor || !Array.isArray(profile.group)) throw new Error('Power AI requires an energy hardware profile.');
  const literal = (id, channel, value) => ({ id, action: { kind: 'write_number', channel, value } });
  const when = (name, comparison, value) => [{ value: { source: 'number_compare', name, comparison, value }, equals: true }];
  const tasks = [
    literal('output', 'reactor-output', profile.reactor.sustainable_output_per_sec),
    literal('reactor-cooling', 'reactor-cooling', profile.reactor.default_cooling_per_sec),
    { ...literal('cool-reactor', 'reactor-cooling', profile.reactor.max_cooling_per_sec), when: when('reactor_heat_fraction', 'greater_or_equal', 0.5) },
  ];
  for (const group of profile.group) {
    tasks.push(literal(`flow-${group.id}`, `flow-${group.id}`, group.default_flow_per_sec));
    tasks.push(literal(`cooling-${group.id}`, `cooling-${group.id}`, group.default_cooling_per_sec));
    tasks.push({ ...literal(`fill-${group.id}`, `flow-${group.id}`, group.max_flow_per_sec), when: when(`reserve-${group.id}`, 'less', 0.5) });
    tasks.push({ ...literal(`hot-${group.id}`, `cooling-${group.id}`, group.max_cooling_per_sec), when: when(`heat-${group.id}`, 'greater_or_equal', 0.5) });
  }
  return { root:'power', tree:[{ id:'power', initial:'balance', state:[{ id:'balance', task:tasks }] }] };
}
