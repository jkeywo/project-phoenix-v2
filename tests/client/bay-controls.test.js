// @vitest-environment jsdom
import {afterEach,expect,it,vi} from 'vitest';
import {createSemanticActionRegistry} from '../../gui/semantic-action-registry.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import {BAY_ACTION_IDS,registerBayActions} from '../../gui/stations/bay-actions.js';
import {mountBayControls} from '../../gui/components/bay-controls.js';
import {mountSystemStatus} from '../../gui/components/ph-system-status.js';
import {dispatchConsoleAction} from '../../gui/action-map.js';
const view=()=>({controllable:true,can_launch:true,can_recover:true,can_cancel:true,capacity_space:2,occupied_space:1,stored:[{id:'own-stored-id',name:'Visible probe',space:1}],contacts:[{contact:'measured-alias',observed_tick:7}],progress:250,paused:false,ready:false,outcome:null});
const status=()=>({system_id:'actual-bay',display_name:'Bay',availability:'normal',reasons:[],delegated:false,bay:view()});
afterEach(()=>{delete window.activateSemanticAction;document.body.replaceChildren();});
it('sends the exact current own roster/contact choices through correlated System admission',()=>{
 const state={system_status:[status()]},send=vi.fn();let seq=0;
 const registry=createSemanticActionRegistry({actionFeedback:new ActionFeedbackLifecycle({correlation:()=>`bay-${++seq}`})});
 registerBayActions(registry,{getState:()=>state,sendAction:send});
 const activate=(kind,detail)=>registry.activate(BAY_ACTION_IDS[kind],{context:'bay',detail});
 activate('launch',{owner:'actual-bay',craft:'own-stored-id'});activate('recover',{owner:'actual-bay',contact:'measured-alias'});activate('cancel',{owner:'actual-bay'});
 const wire=vi.fn();for(const[action,fields]of send.mock.calls)dispatchConsoleAction({action,...fields},wire,vi.fn());
 expect(wire.mock.calls.map(row=>row[1])).toEqual([
 expect.objectContaining({target:'actual-bay',correlation:'bay-1',payload:{type:'LaunchCraft',data:{craft:'own-stored-id'}}}),
 expect.objectContaining({target:'actual-bay',payload:{type:'RecoverCraft',data:{contact:'measured-alias'}}}),
 expect.objectContaining({target:'actual-bay',payload:{type:'CancelCraftHandling'}})]);
 activate('launch',{owner:'actual-bay',craft:'hidden-remote-uuid'});activate('recover',{owner:'actual-bay',contact:'actual-remote-uuid'});activate('cancel',{owner:'foreign-bay'});
 state.system_status[0].delegated=true;activate('cancel',{owner:'actual-bay'});
 state.system_status[0].delegated=false;state.system_status[0].bay.controllable=false;activate('launch',{owner:'actual-bay',craft:'own-stored-id'});
 expect(send).toHaveBeenCalledTimes(3);
});
it('retains focused choices across progress updates, hides delegated controls and never shows technical identities',()=>{
 const widget=mountBayControls(document);document.body.append(widget.node);window.activateSemanticAction=vi.fn();
 widget.update('actual-bay',view());const select=widget.node.querySelector('select'),option=select.options[0];select.focus();
 widget.update('actual-bay',{...view(),progress:500});expect(widget.node.querySelector('select')).toBe(select);expect(select.options[0]).toBe(option);expect(document.activeElement).toBe(select);
 expect(widget.node.textContent).toContain('Visible probe');expect(widget.node.textContent).not.toContain('own-stored-id');expect(widget.node.textContent).not.toContain('measured-alias');
 const [launch,recover,cancel]=widget.node.querySelectorAll('button');launch.click();recover.click();cancel.click();
 expect(window.activateSemanticAction).toHaveBeenCalledTimes(3);
 widget.update('actual-bay',{...view(),contacts:[]});expect(recover.disabled).toBe(true);expect(launch.disabled).toBe(false);
 widget.update('actual-bay',view(),true);expect([...widget.node.querySelectorAll('button')].every(button=>button.hidden&&button.disabled)).toBe(true);widget.dispose();
});
it('mounts the same controls in common System details and readonly delegated summaries',()=>{
 const widget=mountSystemStatus(document);widget.update([status()]);document.querySelector('[data-system-id="actual-bay"]').click();expect(document.querySelector('[data-craft-bay]')).not.toBeNull();
 widget.update([{...status(),delegated:true}]);document.querySelector('[data-delegated-summary]').click();expect([...document.querySelector('[data-craft-bay]').querySelectorAll('button')].every(button=>button.hidden&&button.disabled)).toBe(true);widget.dispose();
});
