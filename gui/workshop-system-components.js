import { t } from './strings.js';

/** A local composition draft. Only Apply crosses the accepted source/history gate. */
export function mountSystemComponents({ root, apply }) {
  const doc = root.ownerDocument;
  const node = (tag, key, attrs = {}) => {
    const el = doc.createElement(tag);
    if (key) el.textContent = t(`workshop.ship.component.${key}`);
    for (const [name, value] of Object.entries(attrs)) el.setAttribute(name, value);
    return el;
  };
  const section = node('fieldset', null, { id: 'workshop-system-components' });
  const list = node('select', null, { id: 'workshop-component-list', size: '4' });
  const id = node('input', null, { id: 'workshop-component-id' });
  const kind = node('select', null, { id: 'workshop-component-kind' });
  const fields = node('div', null, { id: 'workshop-component-fields' });
  const channels = node('div', null, { id: 'workshop-component-channels' });
  const materialise = node('input', null, { id: 'workshop-component-materialise', type: 'checkbox' });
  const label = (el, key) => node('label', key, { for: el.id });
  const button = (key, action) => { const el = node('button', key, { id: `workshop-component-${key}`, type: 'button' }); el.onclick = action; return el; };
  const option = value => { const el = node('option'); el.value = value; el.textContent = value; return el; };
  let owner = null, schema = null, draft = { components: [], channels: [] }, fingerprint = '';
  const initial = system => ({ components: structuredClone(system?.component?.length ? system.component : system?.kind === 'access-gate'
    ? [{ id: 'hatch', kind: 'access_boundary', sealed_channel: 'sealed', transition_secs: 1 }] : []),
    channels: structuredClone(system?.channel?.length ? system.channel : system?.kind === 'access-gate'
      ? [{ name: 'sealed', kind: 'boolean' }] : [{ name: 'enabled', kind: 'boolean' }, { name: 'target', kind: 'entity_target' }]) });
  const value = () => ({ ...structuredClone(draft), materialise: materialise.checked });
  const commit = button('apply', () => apply(() => ({ type: 'system-composition-set', id: owner.id, ...value() })));
  const selected = () => draft.components[Number(list.value)];
  const paintFields = () => {
    const row = selected(); fields.replaceChildren(); id.value = row?.id || ''; kind.value = row?.kind || Object.keys(schema?.component_fields || {})[0] || '';
    for (const field of schema?.component_fields?.[row?.kind] || []) {
      const input = node('input', null, { id: `workshop-component-field-${field.name.replaceAll('.', '-')}` });
      const numeric = ['number', 'integer'].includes(field.kind);
      input.type = numeric ? 'number' : 'text'; if (numeric) input.step = field.kind === 'integer' ? '1' : 'any';
      input.value = field.name.split('.').reduce((obj, key) => obj?.[key], row) ?? '';
      input.oninput = () => {
        const parts = field.name.split('.'); let target = row;
        for (const part of parts.slice(0, -1)) target = target[part] ||= {};
        target[parts.at(-1)] = numeric ? Number(input.value) : input.value;
      };
      const name = node('label', null, { for: input.id }); name.textContent = `${field.name} (${field.kind})`;
      fields.append(name, input);
    }
  };
  const paint = (index = Number(list.value) || 0) => {
    list.replaceChildren(...draft.components.map((row, i) => { const el = option(String(i)); el.textContent = `${row.id} (${row.kind})`; return el; }));
    list.value = String(Math.min(index, draft.components.length - 1));
    channels.replaceChildren();
    for (const channel of draft.channels) {
      const input = node('input', null, { 'aria-label': channel.kind }); input.value = channel.name;
      input.oninput = () => {
        const old = channel.name; channel.name = input.value;
        for (const component of draft.components) for (const field of schema?.component_fields?.[component.kind] || []) {
          if (field.kind === `${channel.kind}_channel` && component[field.name] === old) component[field.name] = channel.name;
        }
        paintFields();
      };
      const type = node('span'); type.textContent = channel.kind; channels.append(type, input);
    }
    paintFields();
  };
  id.oninput = () => { const row = selected(); if (!row) return; const old = row.id; row.id = id.value;
    for (const component of draft.components) if (component.power_input === old) component.power_input = row.id;
    list.selectedOptions[0].textContent = `${row.id} (${row.kind})`; };
  list.onchange = paintFields;
  const add = button('add', () => { const base = kind.value.replaceAll('_', '-'); let next = base, suffix = 2;
    while (draft.components.some(row => row.id === next)) next = `${base}-${suffix++}`;
    draft.components.push({ id: next, kind: kind.value }); paint(draft.components.length - 1); });
  const remove = button('remove', () => { draft.components.splice(Number(list.value), 1); paint(); });
  const move = direction => { const index = Number(list.value), next = index + direction; if (next < 0 || next >= draft.components.length) return;
    [draft.components[index], draft.components[next]] = [draft.components[next], draft.components[index]]; paint(next); };
  const up = button('up', () => move(-1)), down = button('down', () => move(1));
  const reset = button('reset', () => { draft = initial(owner); materialise.checked = false; paint(); });
  section.append(node('legend', 'title'), node('p', 'draft'), label(list, 'title'), list, label(id, 'id'), id, label(kind, 'kind'), kind,
    add, remove, up, down, fields, node('h4', 'channels'), channels, label(materialise, 'materialise'), materialise, reset, commit);
  root.append(section);
  return { value, refresh(system, currentSchema, disabled) {
    schema = currentSchema; owner = system;
    const next = JSON.stringify([system?.id, system?.kind, system?.component, system?.channel, currentSchema?.component_fields]);
    if (next !== fingerprint) { fingerprint = next; draft = initial(system); materialise.checked = false;
      const allowed = system?.kind === 'access-gate' ? ['access_boundary'] : ['dock_mating', 'power_input'];
      kind.replaceChildren(...Object.keys(schema?.component_fields || {}).filter(key => allowed.includes(key)).map(option)); paint(); }
    section.disabled = disabled || !system?.local || !['dock', 'access-gate'].includes(system.kind);
    commit.disabled = !system?.id;
  } };
}
