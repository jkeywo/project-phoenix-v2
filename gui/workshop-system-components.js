import { t } from './strings.js';
import { turretKinds, initialTurret, initialIllumination, initialJamming, initialProtection, initialBay } from '../editor/workshop-system-composition.js';

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
  let owner = null, schema = null, draft = { components: [], channels: [] }, fingerprint = '', documentIdentity = null;
  const editability = () => {
    for (const control of section.querySelectorAll('input,select,button')) {
      control.disabled = !owner?.local && !materialise.checked && control !== materialise;
    }
    commit.disabled ||= !owner?.id;
  };
  const initial = system => system?.kind === 'signal-protection' && !system?.component?.length ? initialProtection() : system?.kind === 'craft-bay' && !system?.component?.length ? initialBay() : ['illumination','jamming'].includes(system?.kind) && !system?.component?.length ? (system.kind==='jamming'?initialJamming():initialIllumination()) : ({ components: structuredClone(system?.component?.length ? system.component : system?.kind === 'access-gate'
    ? [{ id: 'hatch', kind: 'access_boundary', sealed_channel: 'sealed', transition_secs: 1 }] : turretKinds.includes(system?.kind) ? initialTurret() : []),
    channels: structuredClone(system?.channel?.length ? system.channel : system?.kind === 'access-gate'
      ? [{ name: 'sealed', kind: 'boolean' }] : system?.kind === 'dock' ? [{ name: 'enabled', kind: 'boolean' }, { name: 'target', kind: 'entity_target' }] : []) });
  const value = () => { if ([...section.querySelectorAll('input,select')].some(control => !control.checkValidity())) throw new Error('Enter a valid structured value before Apply.'); return { ...structuredClone(draft), materialise: materialise.checked }; };
  const commit = button('apply', () => apply(() => ({ type: 'system-composition-set', id: owner.id, ...value() })));
  const selected = () => draft.components[Number(list.value)];
  const paintFields = () => {
    const row = selected(); fields.replaceChildren(); id.value = row?.id || ''; kind.value = row?.kind || Object.keys(schema?.component_fields || {})[0] || '';
    for (const field of schema?.component_fields?.[row?.kind] || []) {
      const input = node('input', null, { id: `workshop-component-field-${field.name.replaceAll('.', '-')}` });
      const numeric = ['number', 'integer'].includes(field.kind);
      input.type = field.kind === 'boolean' ? 'checkbox' : numeric ? 'number' : 'text'; if (numeric) input.step = field.kind === 'integer' ? '1' : 'any';
      const value = field.name.split('.').reduce((obj, key) => obj?.[key], row);
      if (field.kind === 'boolean') input.checked = value === true; else input.value = field.kind === 'json' ? JSON.stringify(value ?? {}) : value ?? '';
      input.oninput = () => {
        const parts = field.name.split('.'); let target = row;
        for (const part of parts.slice(0, -1)) target = target[part] ||= {};
        if (field.kind === 'json') {
          try {target[parts.at(-1)] = JSON.parse(input.value); input.setCustomValidity('');}
          catch {input.setCustomValidity('Enter valid JSON.');}
        } else target[parts.at(-1)] = field.kind === 'boolean' ? input.checked : numeric ? Number(input.value) : input.value;
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
      const type = node('span'); type.textContent = channel.kind;
      const initial = node('input', null, {'aria-label': `initial ${channel.name}`});
      initial.value = channel.initial == null ? '' : JSON.stringify(channel.initial);
      initial.oninput = () => {
        if (!initial.value.trim()) {delete channel.initial; initial.setCustomValidity(''); return;}
        try {channel.initial = JSON.parse(initial.value); initial.setCustomValidity('');}
        catch {initial.setCustomValidity('Enter valid JSON.');}
      };
      channels.append(type, input, initial);
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
  const reset = button('reset', () => { draft = initial(owner); materialise.checked = false; paint(); editability(); });
  materialise.addEventListener('change', editability);
  section.append(node('legend', 'title'), node('p', 'draft'), label(list, 'title'), list, label(id, 'id'), id, label(kind, 'kind'), kind,
    add, remove, up, down, fields, node('h4', 'channels'), channels, label(materialise, 'materialise'), materialise, reset, commit);
  root.append(section);
  return { value, refresh(system, currentSchema, disabled, currentDocument = null) {
    if (currentDocument !== documentIdentity) {documentIdentity = currentDocument; fingerprint = '';}
    schema = currentSchema; owner = system;
    const next = JSON.stringify([system?.id, system?.kind, system?.component, system?.channel, currentSchema?.component_fields]);
    if (next !== fingerprint) { fingerprint = next; draft = initial(system); materialise.checked = false;
      const allowed = system?.kind === 'signal-protection' ? ['signal_protection', 'power_input'] : system?.kind === 'craft-bay' ? ['craft_bay', 'power_input'] : ['illumination','jamming'].includes(system?.kind) ? [`${system.kind}_emitter`, 'power_input', 'turret'] : system?.kind === 'access-gate' ? ['access_boundary'] : turretKinds.includes(system?.kind) ? ['turret', 'power_input'] : ['dock_mating', 'power_input'];
      kind.replaceChildren(...Object.keys(schema?.component_fields || {}).filter(key => allowed.includes(key)).map(option)); paint(); }
    section.disabled = disabled || !['dock', 'access-gate', 'illumination', 'jamming', 'signal-protection', 'craft-bay', ...turretKinds].includes(system?.kind);
    editability();
  } };
}
