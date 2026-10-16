// Authoring defaults only. Runtime executes the saved, validated tree verbatim.
const fact = name => ({ source: 'fact', name });
const condition = (value, equals = true) => ({ value, equals });
const number = (name, comparison, value) => condition({ source: 'number_compare', name, comparison, value });
const write = (id, channel, value, when = []) => ({ id, when, action: { kind: 'write', channel, value } });
const literal = value => ({ source: 'literal', value });

export function initialShieldTree(recoveryTargets = []) {
  const targets = [...new Set(recoveryTargets)];
  const task = [
    write('raise', 'raised', literal(true)),
    write('focus', 'focus', fact('preferred_focus')),
  ];
  targets.forEach((target, index) => {
    const channel = `recover-${target}`;
    task.push(write(`retain-${index}`, channel, fact(`recovering-${target}`)));
    task.push(write(`recover-${index}`, channel, literal(true), [
      condition(fact('preferred_protection'), false),
      condition(fact(`target-visible-${target}`)),
      condition(fact(`target-preferred-${target}`)),
      condition(fact(`target-ready-${target}`)),
      number('strength_fraction', 'greater_or_equal', 0.85),
      number(`target-strength-${target}`, 'less', 0.4),
      number(`target-reserve-${target}`, 'less', 0.75),
    ]));
    task.push(write(`protect-donor-${index}`, channel, literal(false), [
      number('strength_fraction', 'less_or_equal', 0.6),
    ]));
    task.push(write(`restored-${index}`, channel, literal(false), [
      number(`target-strength-${target}`, 'greater_or_equal', 0.8),
    ]));
    for (const [stop, name] of [`target-visible-${target}`, `target-preferred-${target}`, `target-ready-${target}`].entries()) {
      task.push(write(`stop-${index}-${stop}`, channel, literal(false), [condition(fact(name), false)]));
    }
  });
  return { root: 'shields', tree: [{ id: 'shields', initial: 'operate', state: [{ id: 'operate', task }] }] };
}
