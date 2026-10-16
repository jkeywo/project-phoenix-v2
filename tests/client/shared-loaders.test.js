// @vitest-environment jsdom
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import {it,expect,vi} from 'vitest';
import {ACTION_MAP} from '../../gui/action-map.js';
import {buildWeaponsConsoleState} from '../../gui/console-state.js';
import {createTacticalActionRegistry,TACTICAL_LOADER_PRIORITY_ACTION_ID} from '../../gui/stations/tactical-actions.js';
import '../../gui/components/ph-torpedo-controls.js';
it('carries only received own-tube handling and never turns a feed into control',()=>{
 const owner='torpedo-tube-fore-port';const state={weaponsTubes:[{id:'fore_port'},{id:'aft'}],blackboards:{[owner]:{loader:{group:'forward',work_rate:1,priority:100,waiting:'shared_handling'}}},blackboardKinds:{[owner]:'TorpedoTube'},controlSystemIds:[],controlSources:{[owner]:'Human'}};
 const view=buildWeaponsConsoleState(state,[owner]);expect(view.tubes[0].loader).toMatchObject({priority:100,controllable:false});expect(view.tubes[1].loader).toBeUndefined();
 state.controlSystemIds=[owner];expect(buildWeaponsConsoleState(state,[owner]).tubes[0].loader.controllable).toBe(true);
});
it('semantic priority uses current permitted ownership and closed correlated wire',()=>{
 let sequence=0;let state={tubes:[{id:'fore_port',loader:{owner:'torpedo-tube-fore-port',controllable:true}}]};const sendAction=vi.fn(),registry=createTacticalActionRegistry({getState:()=>state,sendAction,actionFeedback:new ActionFeedbackLifecycle({now:()=>123,correlation:()=>`loader-action-${++sequence}`})});
 expect(registry.activate(TACTICAL_LOADER_PRIORITY_ACTION_ID,{context:'tactical',detail:{tube:'fore_port',priority:4}})).toMatchObject({handled:true});
 expect(sendAction).toHaveBeenCalledWith('set_torpedo_load_priority',expect.objectContaining({control_system_id:'torpedo-tube-fore-port',priority:4}));
 state.tubes[0].loader.controllable=false;expect(registry.activate(TACTICAL_LOADER_PRIORITY_ACTION_ID,{context:'tactical',detail:{tube:'fore_port',priority:1}})).toMatchObject({handled:false});
 const send=vi.fn();ACTION_MAP.set_torpedo_load_priority({control_system_id:'torpedo-tube-fore-port',priority:4,correlation:'owned'},send);expect(send).toHaveBeenCalledWith('ControlSystemCorrelated',{correlation:'owned',target:'torpedo-tube-fore-port',payload:{type:'SetTorpedoLoadPriority',data:{priority:4}}});
 ACTION_MAP.set_torpedo_load_priority({control_system_id:'torpedo-tube-fore-port',priority:65536},send);expect(send).toHaveBeenCalledTimes(1);
});
it('actual tube widget exposes priority and waiting without per-round approval; retired controls cannot act',()=>{
 const widget=document.createElement('ph-torpedo-controls');document.body.append(widget);widget.sendAction=vi.fn();
 widget.state={tubes:[{id:'fore_port',target_count:1,volley_max:2,loader:{owner:'torpedo-tube-fore-port',group:'forward',work_rate:1,priority:4,waiting:'shared_handling',controllable:true}}]};
 const input=widget.shadowRoot.querySelector('.loader-priority input');expect(input.disabled).toBe(false);expect(input.value).toBe('4');expect(widget.shadowRoot.textContent).toContain('Waiting for shared handling');
 input.value='2';input.dispatchEvent(new Event('change'));expect(widget.sendAction).toHaveBeenCalledWith('set_torpedo_load_priority',expect.objectContaining({priority:2}));
 widget.state={tubes:[]};input.dispatchEvent(new Event('change'));expect(widget.sendAction).toHaveBeenCalledTimes(1);widget.remove();
});
