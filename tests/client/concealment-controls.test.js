// @vitest-environment jsdom
import {afterEach,expect,it,vi} from 'vitest';
import {createSemanticActionRegistry} from '../../gui/semantic-action-registry.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import {CONCEALMENT_ENABLED_ACTION,registerConcealmentActions} from '../../gui/stations/concealment-actions.js';
import {mountConcealmentControls} from '../../gui/components/concealment-controls.js';
const view=()=>({enabled:true,active:true,paid_per_mille:500,perceptibility_per_mille:750,
 operating_emission:'<img src=x onerror=attack()>',signature:'weapon',controllable:true});
afterEach(()=>{delete window.activateSemanticAction;document.body.replaceChildren();});
it('admits only the current own System and rechecks delegated and withdrawn controls',()=>{
 const state={system_status:[{system_id:'field',delegated:false,concealment:view()}]},send=vi.fn();
 let sequence=0;
 const registry=createSemanticActionRegistry({actionFeedback:new ActionFeedbackLifecycle({correlation:()=> `field-${++sequence}`})});
 registerConcealmentActions(registry,{getState:()=>state,sendAction:send});
 const activate=detail=>registry.activate(CONCEALMENT_ENABLED_ACTION.id,{context:'concealment',detail});
 activate({owner:'field',enabled:false});
 expect(send).toHaveBeenCalledWith('concealment_enabled',expect.objectContaining({control_system_id:'field',enabled:false}));
 activate({owner:'foreign',enabled:true});activate({owner:'field',enabled:1});
 state.system_status[0].delegated=true;activate({owner:'field',enabled:true});
 state.system_status[0].delegated=false;state.system_status[0].concealment.controllable=false;activate({owner:'field',enabled:true});
 state.system_status=[];activate({owner:'field',enabled:true});expect(send).toHaveBeenCalledTimes(1);
});
it('shows paid work and literal own emission without a detection-success signal',()=>{
 const widget=mountConcealmentControls(document);document.body.append(widget.node);window.activateSemanticAction=vi.fn();
 widget.update('field',view());expect(widget.node.querySelector('img')).toBeNull();
 expect(widget.node.textContent).toContain(view().operating_emission);expect(widget.node.textContent).toContain('50.0');
 const paid=widget.node.querySelector('p').textContent;widget.node.querySelector('button').click();
 expect(window.activateSemanticAction).toHaveBeenCalledWith(CONCEALMENT_ENABLED_ACTION.id,expect.objectContaining({detail:{owner:'field',enabled:false}}));
 widget.update('field',{...view(),active:false});expect(widget.node.querySelector('p').textContent).not.toBe(paid);
 widget.update('field',view(),true);expect(widget.node.querySelector('button').disabled).toBe(true);expect(widget.node.querySelector('button').hidden).toBe(true);
 widget.update('field',null);expect(widget.node.hidden).toBe(true);widget.dispose();
});
