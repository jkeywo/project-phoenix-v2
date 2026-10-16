// @vitest-environment jsdom
import {afterEach,expect,it,vi} from 'vitest';
import {normalizeCommsRequest,normalizeCraftReception,requestStatus} from '../../gui/comms-requests.js';
import {mountBayControls} from '../../gui/components/bay-controls.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import {createSemanticActionRegistry} from '../../gui/semantic-action-registry.js';
import {BAY_ACTION_IDS,registerBayActions} from '../../gui/stations/bay-actions.js';
import {createCommsActionRegistry,COMMS_SEND_REQUEST_ACTION_ID} from '../../gui/stations/comms-actions.js';
import {buildCommsConsoleState} from '../../gui/console-state.js';
import '../../gui/components/ph-comms-text-composer.js';
const order=()=>({request:{origin_id:'internal-order',source_ship:'origin',recipient_ship:'receiver',emitted_tick:2,heard_tick:3,request:{receive_craft:{craft:'private-craft'}}},decision_tick:4});
const view=()=>({controllable:true,can_recover:true,stored:[],contacts:[{contact:'earned-alias',observed_tick:9}],capacity_space:2,occupied_space:0,
 receptions:[{source_name:'Visible source',order:order(),contact:'earned-alias'}]});
afterEach(()=>{document.body.replaceChildren();delete window.activateSemanticAction;});
it('accepts only closed reception values and never turns consent into a status report',()=>{
 expect(normalizeCommsRequest({receive_craft:{craft:'private-craft'}})).toEqual({receive_craft:{craft:'private-craft'}});
 for(const value of [{receive_craft:{craft:'x',job:1}},{receive_craft:{craft:'x\n'}},{receive_craft:{craft:''}}])expect(normalizeCommsRequest(value)).toBeNull();
 expect(normalizeCraftReception(order())).toEqual(order());
 for(const value of [{...order(),job:1},{...order(),decision_tick:2},{...order(),request:{...order().request,formation_sequence:1}}])expect(normalizeCraftReception(value)).toBeNull();
 const text=requestStatus({transmission:{content:{kind:'request',request:order().request.request},received_tick:3},request_progress:{facts:{decision:{choice:'accept',tick:4}}}});
 expect(text).not.toContain('private-craft');expect(text).not.toContain('internal-order');expect(text).not.toContain('physically received');
});
it('uses exact current accepted order plus earned alias through the ordinary Bay action',()=>{
 const state={system_status:[{system_id:'bay',delegated:false,bay:view()}]},send=vi.fn();
 const registry=createSemanticActionRegistry({actionFeedback:new ActionFeedbackLifecycle()});registerBayActions(registry,{getState:()=>state,sendAction:send});
 const detail={owner:'bay',contact:'earned-alias',reception:order()};
 registry.activate(BAY_ACTION_IDS.recover,{context:'bay',detail});expect(send).toHaveBeenCalledTimes(1);
 expect(send.mock.calls[0][1]).toMatchObject({control_system_id:'bay',contact:'earned-alias',reception:order()});
 state.system_status[0].bay.receptions[0].order.decision_tick=5;
 registry.activate(BAY_ACTION_IDS.recover,{context:'bay',detail});expect(send).toHaveBeenCalledTimes(1);
});
it('shows attributed agreement and waiting without IDs, requires own contact and refuses held replacement',()=>{
 const widget=mountBayControls(document);document.body.append(widget.node);window.activateSemanticAction=vi.fn();
 widget.update('bay',view());const button=[...widget.node.querySelectorAll('button')].at(-1);
 expect(widget.node.textContent).toContain('Visible source');expect(widget.node.textContent).not.toMatch(/private-craft|internal-order|earned-alias/);
 button.dispatchEvent(new Event('pointerdown'));const changed=view();changed.receptions[0].order.decision_tick=5;widget.update('bay',changed);button.click();expect(window.activateSemanticAction).not.toHaveBeenCalled();
 widget.update('bay',view());button.click();expect(window.activateSemanticAction).toHaveBeenCalledTimes(1);
 const waiting=view();waiting.receptions[0].contact=null;widget.update('bay',waiting);expect(button.disabled).toBe(true);expect(widget.node.textContent).toContain('Waiting');
 widget.update('bay',view(),true);expect(button.hidden).toBe(true);expect(button.disabled).toBe(true);
});
it('Comms proposes only its current explicitly published own stored craft with no raw IDs on screen',()=>{
 let state={channels:[{id:'public'}],contacts:[{uuid:'receiver',name:'Outpost'}],messages:[],reception_craft:[{id:'private-craft',name:'Rescue pod',space:1}],comms_auto:false};
 const send=vi.fn(),registry=createCommsActionRegistry({getState:()=>state,sendAction:send,actionFeedback:new ActionFeedbackLifecycle()});
 const detail={channel:'public',audience:{kind:'contact',target_uuid:'receiver'},request:{receive_craft:{craft:'private-craft'}}};
 expect(registry.activate(COMMS_SEND_REQUEST_ACTION_ID,{context:'comms',detail}).handled).toBe(true);
 state={...state,reception_craft:[]};expect(registry.activate(COMMS_SEND_REQUEST_ACTION_ID,{context:'comms',detail}).handled).toBe(false);expect(send).toHaveBeenCalledTimes(1);
 const el=document.createElement('ph-comms-text-composer');document.body.append(el);el.state={...state,reception_craft:[{id:'private-craft',name:'Rescue pod'}]};
 expect(el.shadowRoot.textContent).toContain('Rescue pod');expect(el.shadowRoot.textContent).not.toContain('private-craft');
});

it('carries only the selected typed Comms publication through the real Console builder and request action',()=>{
 const craft={id:'private-craft',name:'Rescue pod',space:1};
 const sim={blackboardKinds:{'radio-2':'Comms',bay:'Bay'},
  blackboards:{'radio-2':{channels:[{id:'public'}],contacts:[{uuid:'receiver',name:'Outpost'}],reception_craft:[craft]},
   bay:{stored:[{id:'unshared-craft',name:'Private hold'}]}},controlSources:{'radio-2':'Human'}};
 const state=buildCommsConsoleState(sim,['radio-2']);
 expect(state.reception_craft).toEqual([craft]);expect(state.comms_system_id).toBe('radio-2');
 const send=vi.fn(),registry=createCommsActionRegistry({getState:()=>state,sendAction:send,actionFeedback:new ActionFeedbackLifecycle()});
 const detail={channel:'public',audience:{kind:'contact',target_uuid:'receiver'},request:{receive_craft:{craft:craft.id}}};
 expect(registry.activate(COMMS_SEND_REQUEST_ACTION_ID,{context:'comms',detail}).handled).toBe(true);
 expect(send.mock.calls[0][1]).toMatchObject({control_system_id:'radio-2',request:detail.request});
 delete sim.blackboards['radio-2'].reception_craft;
 expect(buildCommsConsoleState(sim,['radio-2']).reception_craft).toEqual([]);
 expect(buildCommsConsoleState({}).reception_craft ?? []).toEqual([]);
});

it('invalidates retired reception buttons and prevents a disposed widget being revived',()=>{
 const widget=mountBayControls(document);document.body.append(widget.node);window.activateSemanticAction=vi.fn();
 widget.update('bay',view());const button=[...widget.node.querySelectorAll('button')].at(-1);
 button.dispatchEvent(new Event('pointerdown'));widget.dispose();widget.update('bay',view());button.click();
 expect(button.disabled).toBe(true);expect(window.activateSemanticAction).not.toHaveBeenCalled();
});
