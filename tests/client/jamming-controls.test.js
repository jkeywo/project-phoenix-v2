// @vitest-environment jsdom
import {afterEach,expect,it,vi} from 'vitest';
import {createSemanticActionRegistry} from '../../gui/semantic-action-registry.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import {JAMMING_ACTION_IDS,registerJammingActions} from '../../gui/stations/jamming-actions.js';
import {mountJammingControls} from '../../gui/components/jamming-controls.js';
import {mountSystemStatus} from '../../gui/components/ph-system-status.js';
import {dispatchConsoleAction} from '../../gui/action-map.js';
const view=()=>({enabled:true,target:'public-contact',emitting:true,controllable:true,targets:[{contact:'public-contact',name:'Observed contact'}]});
const status=()=>({system_id:'custom-emitter',display_name:'Emitter',availability:'normal',reasons:[],delegated:false,jamming:view()});
afterEach(()=>{delete window.activateSemanticAction;document.body.replaceChildren();});
it('routes only actual owned emitter choices through correlated generic-channel commands',()=>{
 const state={system_status:[status()]},send=vi.fn();let seq=0;
 const registry=createSemanticActionRegistry({actionFeedback:new ActionFeedbackLifecycle({correlation:()=>`light-${++seq}`})});
 registerJammingActions(registry,{getState:()=>state,sendAction:send});
 const activate=(kind,detail)=>registry.activate(JAMMING_ACTION_IDS[kind],{context:'jamming',detail});
 activate('target',{owner:'custom-emitter',target_uuid:'public-contact'});
 activate('enabled',{owner:'custom-emitter',enabled:false});
 activate('target',{owner:'custom-emitter',target_uuid:null});
 const wire=vi.fn();for(const[action,fields]of send.mock.calls)dispatchConsoleAction({action,...fields},wire,vi.fn());
 expect(wire.mock.calls.map(row=>row[1])).toEqual([
 expect.objectContaining({target:'custom-emitter',correlation:'light-1',payload:{type:'SetJammingTarget',data:{target_uuid:'public-contact'}}}),
 expect.objectContaining({target:'custom-emitter',payload:{type:'SetJammingEnabled',data:{enabled:false}}}),
 expect.objectContaining({target:'custom-emitter',payload:{type:'SetJammingTarget',data:{target_uuid:null}}}),
 ]);
 activate('target',{owner:'custom-emitter',target_uuid:'private-id'});activate('enabled',{owner:'foreign-emitter',enabled:true});
 state.system_status[0].delegated=true;activate('enabled',{owner:'custom-emitter',enabled:true});
 state.system_status[0].delegated=false;state.system_status[0].jamming.controllable=false;activate('target',{owner:'custom-emitter',target_uuid:null});
 expect(send).toHaveBeenCalledTimes(3);
});
it('keeps focused native target control stable and never claims a receiver result',()=>{
 const widget=mountJammingControls(document);document.body.append(widget.node);window.activateSemanticAction=vi.fn();
 widget.update('custom-emitter',view());const select=widget.node.querySelector('select'),option=select.options[0];select.focus();
 widget.update('custom-emitter',{...view(),emitting:false});
 expect(widget.node.querySelector('select')).toBe(select);expect(select.options[0]).toBe(option);expect(document.activeElement).toBe(select);
 widget.update('custom-emitter',view());expect(widget.node.textContent).toContain('receiver effects unknown');
 const[apply,clear,toggle]=widget.node.querySelectorAll('button');apply.click();toggle.click();clear.click();
 expect(window.activateSemanticAction).toHaveBeenCalledWith(JAMMING_ACTION_IDS.enabled,expect.objectContaining({detail:{owner:'custom-emitter',enabled:false}}));
 widget.update('custom-emitter',{...view(),targets:[]});expect(apply.disabled).toBe(true);expect(toggle.disabled).toBe(false);expect(clear.disabled).toBe(false);
 widget.update('custom-emitter',view(),true);expect([...widget.node.querySelectorAll('button')].every(button=>button.hidden&&button.disabled)).toBe(true);
 widget.dispose();
});
it('mounts independent Comms controls in shared equipment details and readonly delegated summaries',()=>{
 const widget=mountSystemStatus(document);widget.update([status()]);document.querySelector('[data-system-id="custom-emitter"]').click();
 expect(document.querySelector('[data-jamming]')).not.toBeNull();
 widget.update([{...status(),delegated:true}]);document.querySelector('[data-delegated-summary]').click();
 expect([...document.querySelector('[data-jamming]').querySelectorAll('button')].every(button=>button.hidden&&button.disabled)).toBe(true);widget.dispose();
});
