// @vitest-environment jsdom
import { expect, it, vi } from 'vitest';
import { createGmSystemPanel, GM_SYSTEM_CONFIRMATION } from '../../gui/gm-system-panel.js';
import { createGmConfirmationController, createGmConfirmationProfile } from '../../gui/gm-confirmation.js';

function setup({ confirmAction, operator = { id: 'gm-one' } } = {}) {
  document.body.innerHTML = '<p id="gm-system-target"></p><select id="gm-system-select"></select><p id="gm-system-state"></p><button id="gm-system-disable"></button><button id="gm-system-restore"></button><p id="gm-system-feedback"></p>';
  const submit=vi.fn(()=>true), correlation=vi.fn(()=>'request-one'), schedule=vi.fn(()=>7), cancelSchedule=vi.fn();
  const panel=createGmSystemPanel({ doc:document,getOperator:()=>operator,submit,correlation,schedule,cancelSchedule,confirmAction });
  const payload={ entities:[{entity_id:'ship-a',name:'A'},{entity_id:'ship-b',name:'B'}],system_controls:{
    'ship-a':[{system_id:'drive',name:'Drive',gm_disabled:false,available:true}],
    'ship-b':[{system_id:'drive',name:'Drive B',gm_disabled:false,available:true}],
  },system_results:[] };
  panel.update(payload); panel.select(payload.entities[0]);
  document.getElementById('gm-system-select').value='drive';
  document.getElementById('gm-system-select').dispatchEvent(new Event('change'));
  return {panel,payload,submit,correlation,schedule,cancelSchedule};
}
it('captures the selected identity before confirmation and creates Pending only after acceptance',()=>{
  let confirmation; const f=setup({confirmAction:request=>{confirmation=request;return true;}});
  expect(f.panel.choose(true)).toBe(true);
  expect(confirmation).toMatchObject({category:'system.disable',defaultMode:'confirm'});
  expect(f.correlation).not.toHaveBeenCalled(); expect(f.submit).not.toHaveBeenCalled();
  expect(f.panel.state().pending).toBeNull();
  f.panel.select(f.payload.entities[1]);
  expect(confirmation.accept()).toBe(true);
  expect(f.submit).toHaveBeenCalledWith({operator_id:'gm-one',correlation:'request-one',target:'ship-a',system:'drive',disabled:true});
  expect(f.panel.state().controls['ship-a'][0].gm_disabled).toBe(false);
  expect(f.panel.choose(false)).toBe(false);
});
it('shared confirmation preserves a stale captured System for canonical refusal',()=>{
  let controller;
  const f=setup({confirmAction:request=>controller.request(request)});
  controller=createGmConfirmationController({doc:document,
    profile:createGmConfirmationProfile({storage:{getItem:()=>null,setItem:()=>{}}})});
  f.panel.choose(true);
  expect(f.submit).not.toHaveBeenCalled(); expect(f.correlation).not.toHaveBeenCalled();
  f.panel.update({...f.payload,entities:[],system_controls:{}});
  document.querySelector('[data-confirmation-accept]').click();
  expect(f.submit).toHaveBeenCalledWith({operator_id:'gm-one',correlation:'request-one',target:'ship-a',system:'drive',disabled:true});
  expect(f.panel.state().pending).not.toBeNull();
  f.panel.update({...f.payload,entities:[],system_controls:{},system_results:[{
    operator_id:'gm-one',correlation:'request-one',target:'ship-a',action_kind:'system-disable',
    effect_scope:{system:'drive'},outcome:'refused',reason:'unknown-entity',
  }]});
  expect(f.panel.state().pending).toBeNull();
  expect(document.getElementById('gm-system-feedback').dataset.state).toBe('refused');
  controller.destroy();
});
it('cancelling shared System confirmation sends no command or correlation',()=>{
  let controller;
  const f=setup({confirmAction:request=>controller.request(request)});
  controller=createGmConfirmationController({doc:document,
    profile:createGmConfirmationProfile({storage:{getItem:()=>null,setItem:()=>{}}})});
  f.panel.choose(true);
  document.querySelector('[data-confirmation-cancel]').click();
  expect(f.submit).not.toHaveBeenCalled(); expect(f.correlation).not.toHaveBeenCalled();
  controller.destroy();
});
it('reset invalidates a previously opened confirmation',()=>{
  let confirmation; const f=setup({confirmAction:request=>{confirmation=request;return true;}});
  f.panel.choose(true); f.panel.reset(); f.panel.update(f.payload);
  expect(confirmation.accept()).toBe(false); expect(f.submit).not.toHaveBeenCalled();
});
it.each(['applied','no-op','refused'])('only matching System and absolute action settle %s',outcome=>{
  const f=setup(); f.panel.choose(false);
  const row={operator_id:'gm-one',correlation:'request-one',target:'ship-a',action_kind:'system-restore',effect_scope:{system:'wrong'},outcome};
  f.panel.update({...f.payload,system_results:[row]}); expect(f.panel.state().pending).not.toBeNull();
  f.panel.update({...f.payload,system_results:[{...row,effect_scope:{system:'drive'}}]});
  expect(f.panel.state().pending).toBeNull(); expect(f.cancelSchedule).toHaveBeenCalledWith(7);
  expect(document.getElementById('gm-system-feedback').dataset.state).toBe(outcome==='no-op'?'no_op':outcome);
});
it('renders a restored latch separately from damage availability without inventing healing',()=>{
  const f=setup();
  f.panel.select({...f.payload.entities[0]});
  expect(f.panel.state().system).toBe('drive');
  f.panel.update({...f.payload,system_controls:{'ship-a':[{system_id:'drive',name:'Drive',gm_disabled:true,available:false}]}});
  expect(document.getElementById('gm-system-state').textContent).toBe('server.gm.system.disabled');
  f.panel.update({...f.payload,system_controls:{'ship-a':[{system_id:'drive',name:'Drive',gm_disabled:false,available:false}]}});
  expect(document.getElementById('gm-system-state').textContent).toBe('server.gm.system.offline');
  expect(GM_SYSTEM_CONFIRMATION.restore).toEqual({category:'system.restore',defaultMode:'immediate'});
});
it('crew admission and malformed scope cannot create or settle a request',()=>{
  const f=setup({operator:null}); expect(f.panel.choose(true)).toBe(false);
  expect(f.panel.update({...f.payload,system_results:[{action_kind:'system-disable',effect_scope:{station:'helm'},target:'ship-a',operator_id:'gm',correlation:'x',outcome:'applied'}]})).toBe(false);
  expect(f.submit).not.toHaveBeenCalled();
});
