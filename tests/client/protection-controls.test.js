// @vitest-environment jsdom
import {afterEach, expect, it, vi} from 'vitest';
import {createSemanticActionRegistry} from '../../gui/semantic-action-registry.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import {PROTECTION_ACTION_IDS, registerProtectionActions} from '../../gui/stations/protection-actions.js';
import {mountProtectionControls} from '../../gui/components/protection-controls.js';
import {mountSystemStatus} from '../../gui/components/ph-system-status.js';
import {initConsole} from '../../gui/console-core.js';
import {dispatchConsoleAction} from '../../gui/action-map.js';
const view = () => ({enabled:true, priorities:['comms','radar'], capacity:3, active:true,
 protected_systems:['comms'], controllable:true, systems:[
 {system:'comms', display_name:'Radio', cost:2, home_station:'comms'},
 {system:'radar', display_name:'Radar', cost:2, home_station:'sensors'},
 {system:'passive', display_name:'Passive', cost:1, home_station:'sensors'}]});
const row = () => ({system_id:'guard', display_name:'Protection', availability:'normal', reasons:[], delegated:false, signal_protection:view()});
afterEach(() => {delete window.activateSemanticAction; document.body.replaceChildren();});
it('sends exact correlated owner controls and refuses foreign, delegated or malformed priorities', () => {
 const state={system_status:[row()]}, send=vi.fn(); let sequence=0;
 const registry=createSemanticActionRegistry({actionFeedback:new ActionFeedbackLifecycle({correlation:()=>`guard-${++sequence}`})});
 registerProtectionActions(registry,{getState:()=>state,sendAction:send});
 const activate=(kind,detail)=>registry.activate(PROTECTION_ACTION_IDS[kind],{context:'protection',detail});
 activate('priorities',{owner:'guard',systems:['passive','comms']});
 activate('enabled',{owner:'guard',enabled:false});
 const wire=vi.fn(); for(const[action,fields]of send.mock.calls) dispatchConsoleAction({action,...fields},wire,vi.fn());
 expect(wire.mock.calls.map(call=>call[1])).toEqual([
 expect.objectContaining({target:'guard',correlation:'guard-1',payload:{type:'SetSignalProtectionPriorities',data:{systems:['passive','comms']}}}),
 expect.objectContaining({target:'guard',payload:{type:'SetSignalProtectionEnabled',data:{enabled:false}}})]);
 for(const systems of [['secret'],['comms','comms'],Array(65).fill('comms'),null]) activate('priorities',{owner:'guard',systems});
 activate('enabled',{owner:'foreign',enabled:true}); activate('enabled',{owner:'guard',enabled:1});
 state.system_status[0].delegated=true; activate('priorities',{owner:'guard',systems:[]});
 state.system_status[0].delegated=false;state.system_status[0].signal_protection.controllable=false;activate('enabled',{owner:'guard',enabled:true});
 expect(send).toHaveBeenCalledTimes(2);
});
it('retains an ordered local draft across payment updates and expands explicit own Station presets',()=>{
 const widget=mountProtectionControls(document);document.body.append(widget.node);window.activateSemanticAction=vi.fn();widget.update('guard',view());
 const selects=widget.node.querySelectorAll('select'), choices=selects[0], priorities=selects[1], stations=selects[2];
 const button=name=>widget.node.querySelector(`[data-protection-action="${name}"]`);
 priorities.value='radar';priorities.dispatchEvent(new Event('change'));button('up').click();
 choices.value='passive';choices.dispatchEvent(new Event('change'));button('add').click();
 widget.update('guard',{...view(),active:false,protected_systems:[]});
 expect([...priorities.options].map(row=>row.value)).toEqual(['radar','comms','passive']);
 expect(widget.node.querySelector('select')).toBe(choices);button('apply').click();
 expect(window.activateSemanticAction).toHaveBeenLastCalledWith(PROTECTION_ACTION_IDS.priorities,expect.objectContaining({detail:{owner:'guard',systems:['radar','comms','passive']}}));
 stations.value='sensors';stations.dispatchEvent(new Event('change'));button('preset').click();button('apply').click();
 expect(window.activateSemanticAction).toHaveBeenLastCalledWith(PROTECTION_ACTION_IDS.priorities,expect.objectContaining({detail:{owner:'guard',systems:['radar','passive']}}));
 widget.update('guard',{...view(),priorities:['passive']});expect([...priorities.options].map(row=>row.value)).toEqual(['passive']);
 button('enable').click();expect(window.activateSemanticAction).toHaveBeenLastCalledWith(PROTECTION_ACTION_IDS.enabled,expect.objectContaining({detail:{owner:'guard',enabled:false}}));
 widget.update('guard',view(),true);expect([...widget.node.querySelectorAll('button')].every(control=>control.hidden&&control.disabled)).toBe(true);
 widget.update('guard',null);expect(button('enable').disabled).toBe(true);widget.dispose();
});
it('uses shared equipment details for the owned controls and a read-only delegated summary',()=>{
 const widget=mountSystemStatus(document);widget.update([row()]);document.querySelector('[data-system-id="guard"]').click();
 expect(document.querySelector('[data-signal-protection]')).not.toBeNull();
 widget.update([{...row(),delegated:true}]);document.querySelector('[data-delegated-summary]').click();
 expect([...document.querySelector('[data-signal-protection]').querySelectorAll('button')].every(control=>control.disabled&&control.hidden)).toBe(true);widget.dispose();
});

it('registers protection in the actual Console runtime and keeps the exact wire authority',()=>{
 const sent=[];window.__sendAction=json=>sent.push(JSON.parse(json));const runtime=initConsole({name:'sensors',render:()=>{}});
 try {
 window.__updateConsole('sensors',JSON.stringify({system_status:[row()]}));
 window.activateSemanticAction(PROTECTION_ACTION_IDS.priorities,{context:'protection',detail:{owner:'guard',systems:['passive','comms']}});
 expect(sent).toEqual([expect.objectContaining({action:'signal_protection_priorities',control_system_id:'guard',systems:['passive','comms'],semantic_action:PROTECTION_ACTION_IDS.priorities})]);
 window.activateSemanticAction(PROTECTION_ACTION_IDS.enabled,{context:'protection',detail:{owner:'foreign',enabled:true}});expect(sent).toHaveLength(1);
 } finally {runtime.disposeSemanticActions();for(const key of ['__sendAction','__updateConsole','__updateActionFeedback','__updateSemanticActionBindings','activateSemanticAction','sendAction'])delete window[key];}
});
