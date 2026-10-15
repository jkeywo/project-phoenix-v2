/** @vitest-environment jsdom */
import { describe, it, expect, vi } from 'vitest';
import { mountSystemComponents } from '../../gui/workshop-system-components.js';
const schema = { component_fields: { power_input: [], dock_mating: [
  { name: 'enabled_channel', kind: 'boolean_channel' }, { name: 'target_channel', kind: 'entity_target_channel' },
  { name: 'power_input', kind: 'power_input_component' }, { name: 'config.range', kind: 'number' },
] } };
const system = { id: 'dock', kind: 'dock', local: true, component: [
  { id: 'power', kind: 'power_input' }, { id: 'mate', kind: 'dock_mating', enabled_channel: 'enabled', target_channel: 'target', power_input: 'power', config: { range: 15 } },
], channel: [{ name: 'enabled', kind: 'boolean' }, { name: 'target', kind: 'entity_target' }] };
const change = (element, value) => { element.value = value; element.dispatchEvent(new Event('input', { bubbles: true })); };
const el = name => document.getElementById(`workshop-component-${name}`);
describe('staged component composition controls', () => {
  it('offers a closed access-boundary starter and binds its shared seal channel', () => {
    document.body.innerHTML = '<main></main>'; const editor = mountSystemComponents({ root: document.querySelector('main'), apply: vi.fn() });
    editor.refresh({ id: null, kind: 'access-gate', local: true }, { component_fields: { ...schema.component_fields, access_boundary: [
      { name: 'sealed_channel', kind: 'boolean_channel' }, { name: 'transition_secs', kind: 'number' }] } }, false);
    expect(document.querySelector('fieldset').disabled).toBe(false);
    expect([...el('kind').options].map(option => option.value)).toEqual(['access_boundary']);
    expect(editor.value().components).toEqual([{ id: 'hatch', kind: 'access_boundary', sealed_channel: 'sealed', transition_secs: 1 }]);
    change(el('channels').querySelector('input'), 'contain');
    expect(editor.value().components[0].sealed_channel).toBe('contain');
  });
  it('stages edits and shared bindings until a single Apply operation', () => {
    document.body.innerHTML = '<main></main>'; const apply = vi.fn();
    const editor = mountSystemComponents({ root: document.querySelector('main'), apply }); editor.refresh(system, schema, false);
    change(el('id'), 'supply');
    const channel = el('channels').querySelector('input[aria-label="boolean"]'); change(channel, 'engage');
    el('list').value = '1'; el('list').dispatchEvent(new Event('change'));
    expect(el('field-power_input').value).toBe('supply'); expect(el('field-enabled_channel').value).toBe('engage');
    change(el('field-config-range'), '30'); el('up').click();
    expect(apply).not.toHaveBeenCalled(); expect(system.component[1].config.range).toBe(15);
    el('apply').click(); const operation = apply.mock.calls[0][0]();
    expect(operation.type).toBe('system-composition-set');
    expect(operation.components[0]).toMatchObject({ id: 'mate', power_input: 'supply', enabled_channel: 'engage', config: { range: 30 } });
    expect(operation.channels[0]).toEqual({ name: 'engage', kind: 'boolean' });
    el('reset').click(); expect(editor.value().components).toEqual(system.component);
  });
  it('requires explicit inherited materialisation and keeps unsupported kinds disabled', () => {
    document.body.innerHTML = '<main></main>'; const editor = mountSystemComponents({ root: document.querySelector('main'), apply: vi.fn() });
    editor.refresh({ ...system, local: false }, schema, false); expect(el('apply').disabled).toBe(true);
    expect(el('materialise').disabled).toBe(false);
    el('materialise').checked = true; el('materialise').dispatchEvent(new Event('change'));
    expect(el('apply').disabled).toBe(false);
    editor.refresh({ ...system, kind: 'sensors' }, schema, false); expect(document.querySelector('fieldset').disabled).toBe(true);
  });
});

it('edits one finite turret using its System intent without inventing channels', () => {
  document.body.innerHTML = '<main></main>'; const apply = vi.fn();
  const editor = mountSystemComponents({ root: document.querySelector('main'), apply });
  const turretSchema = { component_fields: { ...schema.component_fields, turret: [
    { name: 'power_input', kind: 'power_input_component' },
    ...['facing_deg', 'traverse_arc_deg', 'traverse_deg_per_sec', 'aim_tolerance_deg'].map(name => ({ name: 'config.' + name, kind: 'number' })),
  ] } };
  editor.refresh({ id: 'beam', kind: 'phaser_bank', local: true, power_group: 'bank' }, turretSchema, false);
  expect([...el('kind').options].map(option => option.value)).toEqual(['power_input', 'turret']);
  expect(editor.value().channels).toEqual([]);
  el('list').value = '1'; el('list').dispatchEvent(new Event('change'));
  const speed = el('field-config-traverse_deg_per_sec'); speed.focus(); change(speed, '17.5');
  editor.refresh({ id: 'beam', kind: 'phaser_bank', local: true, power_group: 'bank' }, turretSchema, false);
  expect(document.activeElement).toBe(speed); expect(speed.value).toBe('17.5');
  el('apply').click(); const operation = apply.mock.calls[0][0]();
  expect(operation).toMatchObject({ type: 'system-composition-set', id: 'beam', components: [
    { id: 'turret-power', kind: 'power_input' }, { kind: 'turret', power_input: 'turret-power', config: { traverse_deg_per_sec: 17.5 } },
  ], channels: [] });
});

it('stages the first Dock System without pretending it is already an editable owner', () => {
  document.body.innerHTML = '<main></main>'; const apply = vi.fn();
  const editor = mountSystemComponents({ root: document.querySelector('main'), apply });
  editor.refresh({ id: null, kind: 'dock', local: true }, schema, false);
  expect(document.querySelector('fieldset').disabled).toBe(false);
  expect(el('apply').disabled).toBe(true);
  el('kind').value = 'power_input'; el('add').click();
  el('kind').value = 'dock_mating'; el('add').click();
  change(el('field-enabled_channel'), 'enabled'); change(el('field-target_channel'), 'target');
  change(el('field-power_input'), 'power-input'); change(el('field-config-range'), '100');
  expect(editor.value()).toMatchObject({ components: [{ kind: 'power_input' }, { kind: 'dock_mating', enabled_channel: 'enabled', target_channel: 'target', power_input: 'power-input', config: { range: 100 } }],
    channels: [{ name: 'enabled', kind: 'boolean' }, { name: 'target', kind: 'entity_target' }] });
  expect(apply).not.toHaveBeenCalled();
});

it('stages illumination enabled/target bindings with finite component fields and no Tactical selection',()=>{
 document.body.innerHTML='<main></main>';const apply=vi.fn();const editor=mountSystemComponents({root:document.querySelector('main'),apply});
 const illumination={component_fields:{power_input:[],illumination_emitter:[{name:'enabled_channel',kind:'boolean_channel'},{name:'target_channel',kind:'entity_target_channel'},{name:'power_input',kind:'power_input_component'},{name:'config.link_range',kind:'number'}],turret:[]}};
 editor.refresh({id:'science-light',kind:'illumination',local:true,power_group:'science'},illumination,false);
 expect(document.querySelector('fieldset').disabled).toBe(false);
 expect(editor.value().components.map(row=>row.kind)).toEqual(['power_input','illumination_emitter']);
 change(el('channels').querySelector('[aria-label="entity_target"]'),'subject');
 el('list').value='1';el('list').dispatchEvent(new Event('change'));expect(el('field-target_channel').value).toBe('subject');
 change(el('field-config-link_range'),'125');el('apply').click();
 expect(apply.mock.calls[0][0]()).toMatchObject({id:'science-light',components:[{kind:'power_input'},{kind:'illumination_emitter',target_channel:'subject',config:{link_range:125}}],channels:[{kind:'boolean'},{name:'subject',kind:'entity_target'}]});
});

it('edits explicit moving Dock capability with a boolean control rather than text', () => {
  document.body.innerHTML='<main></main>'; const editor=mountSystemComponents({root:document.querySelector('main'),apply:vi.fn()});
  const closed={component_fields:{...schema.component_fields,dock_mating:[...schema.component_fields.dock_mating,{name:'config.moves_body',kind:'boolean'}]}};
  editor.refresh(system,closed,false); el('list').value='1'; el('list').dispatchEvent(new Event('change'));
  const control=el('field-config-moves_body'); expect(control.type).toBe('checkbox'); expect(control.checked).toBe(false);
  control.checked=true; control.dispatchEvent(new Event('input')); expect(editor.value().components[1].config.moves_body).toBe(true);
  control.checked=false; control.dispatchEvent(new Event('input')); expect(editor.value().components[1].config.moves_body).toBe(false);
  document.body.innerHTML = "";
});

it('stages independent jammer evidence/strength/falloff fields through one composition transaction',()=>{
 document.body.innerHTML='<main></main>';const apply=vi.fn(),editor=mountSystemComponents({root:document.querySelector('main'),apply});
 const jamming={component_fields:{power_input:[],jamming_emitter:[{name:'enabled_channel',kind:'boolean_channel'},{name:'target_channel',kind:'entity_target_channel'},{name:'power_input',kind:'power_input_component'},...['strength','maximum_sample_age_secs'].map(name=>({name:'config.'+name,kind:'number'})),{name:'config.falloff',kind:'text'}],turret:[]}};
 editor.refresh({id:'comms-jam',kind:'jamming',local:true,power_group:'radio'},jamming,false);
 expect(document.querySelector('fieldset').disabled).toBe(false);expect(editor.value().components.map(row=>row.kind)).toEqual(['power_input','jamming_emitter']);
 change(el('channels').querySelector('[aria-label="boolean"]'),'radiating');el('list').value='1';el('list').dispatchEvent(new Event('change'));
 expect(el('field-enabled_channel').value).toBe('radiating');change(el('field-config-strength'),'2.5');change(el('field-config-maximum_sample_age_secs'),'1.5');change(el('field-config-falloff'),'smooth');
 expect(apply).not.toHaveBeenCalled();el('apply').click();expect(apply.mock.calls[0][0]()).toMatchObject({id:'comms-jam',components:[{kind:'power_input'},{kind:'jamming_emitter',enabled_channel:'radiating',config:{strength:2.5,maximum_sample_age_secs:1.5,falloff:'smooth'}}]});
});

it('keeps protection costs and typed initials in one draft and resets on same-source document replacement',()=>{
 document.body.innerHTML='<main></main>';const apply=vi.fn(),editor=mountSystemComponents({root:document.querySelector('main'),apply});
 const schema={component_fields:{power_input:[],signal_protection:[{name:'enabled_channel',kind:'boolean_channel'},{name:'priorities_channel',kind:'system_priority_channel'},{name:'power_input',kind:'power_input_component'},{name:'config.capacity',kind:'integer'},{name:'config.strength',kind:'number'},{name:'config.systems',kind:'json'}]}};
 const owner={id:'guard',kind:'signal-protection',local:true,power_group:'science'},first={},replacement={};
 editor.refresh(owner,schema,false,first);change(el('channels').querySelector('[aria-label="system_priority"]'),'ordered');
 const initial=el('channels').querySelector('[aria-label="initial priorities"]');change(initial,'{"kind":"system_priority","value":["sensors"]}');
 el('list').value='1';el('list').dispatchEvent(new Event('change'));
 change(el('field-config-systems'),'bad JSON');expect(()=>editor.value()).toThrow();
 change(el('field-config-systems'),'{"sensors":1}');expect(editor.value().components[1].priorities_channel).toBe('ordered');
 el('apply').click();expect(apply.mock.calls[0][0]()).toMatchObject({id:'guard',components:[{kind:'power_input'},{kind:'signal_protection',config:{systems:{sensors:1}}}],channels:[{initial:{kind:'boolean',value:true}},{name:'ordered',initial:{kind:'system_priority',value:['sensors']}}]});
 editor.refresh(owner,schema,false,first);expect(editor.value().channels[1].name).toBe('ordered');
 editor.refresh(owner,schema,false,replacement);expect(editor.value().channels[1].name).toBe('priorities');expect(editor.value().components[1].config.systems).toEqual({});
});
