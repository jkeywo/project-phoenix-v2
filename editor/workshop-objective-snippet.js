/** A source template, not a second Objective resolver. Save runs the runtime validator. */
const quote = value => JSON.stringify(String(value));

export function objectiveInstanceSnippet({ id, instanceId, text, shipSlots = [], factions = [],
  allPlayerShips = false, completeAfterSeconds = null }, source = '') {
  const identity = value => /^[A-Za-z0-9][A-Za-z0-9_-]*$/.test(value);
  if (!identity(id) || !identity(instanceId) || !/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(text?.trim() || '')) {
    throw new Error('workshop.objective.error_fields');
  }
  if (!allPlayerShips && shipSlots.length === 0 && factions.length === 0) {
    throw new Error('workshop.objective.error_recipient');
  }
  if (shipSlots.some(value => !value || /[\r\n]/.test(value)) || factions.some(value => !value || /[\r\n]/.test(value))
    || new Set(shipSlots).size !== shipSlots.length || new Set(factions).size !== factions.length) {
    throw new Error('workshop.objective.error_selector');
  }
  if (completeAfterSeconds !== null && (!Number.isSafeInteger(completeAfterSeconds) || completeAfterSeconds <= 0)) {
    throw new Error('workshop.objective.error_delay');
  }
  const suffix = `${id}_${instanceId}`.replaceAll('-', '_');
  let functionName = `workshop_objective_${suffix}`;
  for (let index = 2; new RegExp(`\\b${functionName}\\b`).test(source); index += 1) {
    functionName = `workshop_objective_${suffix}_${index}`;
  }
  const fields = [`id: ${quote(id)}`, `instance_id: ${quote(instanceId)}`, `text: ${quote(text.trim())}`];
  if (shipSlots.length) fields.push(`recipient_ship_slots: [${shipSlots.map(quote).join(', ')}]`);
  if (factions.length) fields.push(`recipient_factions: [${factions.map(quote).join(', ')}]`);
  if (allPlayerShips) fields.push('all_player_ships: true');
  const lines = [
    `on_world_loaded(${quote(functionName)});`,
    `fn ${functionName}(ctx) {`,
    '    ctx.effects.add_objective(#{',
    ...fields.map(field => `        ${field},`),
    '    });',
  ];
  if (completeAfterSeconds !== null) {
    lines.push(`    ctx.schedule.in_seconds(${completeAfterSeconds}).complete_objective(${quote(id)}, ${quote(instanceId)});`);
  }
  lines.push('}');
  return lines.join('\n');
}

/** Update an earlier template with the same key, or append a new editable one. */
export function upsertObjectiveInstanceSnippet(values, source) {
  const key = `${values.id}/${values.instanceId}`;
  const start = `// workshop-objective:${key}:start`;
  const end = `// workshop-objective:${key}:end`;
  const opening = source.indexOf(start);
  const closing = opening < 0 ? -1 : source.indexOf(end, opening + start.length);
  if (opening >= 0 && closing < 0) throw new Error('workshop.objective.error_source');
  const without = opening < 0 ? source : source.slice(0, opening) + source.slice(closing + end.length).replace(/^\r?\n/, '');
  const snippet = `${start}\n${objectiveInstanceSnippet(values, without)}\n${end}\n`;
  if (opening >= 0) return source.slice(0, opening) + snippet + source.slice(closing + end.length).replace(/^\r?\n/, '');
  return source + (source.endsWith('\n') ? '\n' : '\n\n') + snippet;
}

/** A ship-scoped modifier action using the runtime's addressed dispatcher. */
export function addressedModifierSnippet({ kind, slot, tag, bonus, shipSlots = [], factions = [],
  allPlayerShips = false, objectiveId = '', instanceId = '', instanceMembers = false }, source = '') {
  if (!['apply_modifier', 'remove_modifier'].includes(kind) || !slot?.trim() || !tag?.trim()
    || instanceMembers && (!objectiveId?.trim() || !instanceId?.trim())) {
    throw new Error('workshop.objective.error_action');
  }
  if (kind === 'apply_modifier' && (!Number.isFinite(bonus) || bonus === 0)) {
    throw new Error('workshop.objective.error_bonus');
  }
  const fields = [`type: ${quote(kind)}`, `slot: ${quote(slot.trim())}`, `tag: ${quote(tag.trim())}`];
  if (kind === 'apply_modifier') fields.push(`bonus: flt(${quote(bonus)})`);
  if (shipSlots.length) fields.push(`recipient_ship_slots: [${shipSlots.map(quote).join(', ')}]`);
  if (factions.length) fields.push(`recipient_factions: [${factions.map(quote).join(', ')}]`);
  if (allPlayerShips) fields.push('all_player_ships: true');
  if (instanceMembers) fields.push(`recipient_objective_instances: [#{ objective_id: ${quote(objectiveId.trim())}, instance_id: ${quote(instanceId.trim())} }]`);
  if (!shipSlots.length && !factions.length && !allPlayerShips && !instanceMembers) {
    fields.push('recipient_ship_slots: []');
  }
  const base = `workshop_addressed_${tag.trim().replace(/[^A-Za-z0-9_]/g, '_')}`;
  let functionName = base;
  for (let index = 2; new RegExp(`\\b${functionName}\\b`).test(source); index += 1) functionName = `${base}_${index}`;
  return [`on_world_loaded(${quote(functionName)});`, `fn ${functionName}(ctx) {`,
    '    ctx.effects.addressed(#{', ...fields.map(field => `        ${field},`), '    });', '}'].join('\n');
}
