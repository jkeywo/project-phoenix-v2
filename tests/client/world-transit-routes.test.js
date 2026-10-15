/** @vitest-environment jsdom */
import {it,expect,vi} from 'vitest';
import {buildTransitRoutes} from '../../gui/transit.js';
import {buildNavigationConsoleState} from '../../gui/console-state.js';
import {consoleReadState} from '../../gui/console-topology.js';
import {createNavigationActionRegistry,NAVIGATION_WORLD_ROUTE_ACTION_ID} from '../../gui/stations/navigation-actions.js';
import {mountNavigationSharingControls} from '../../gui/components/navigation-sharing-controls.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
const route=()=>({id:'outer',label:'known.outer',point:[10000,12,0]});
const state=()=>({blackboardKinds:{'own-nav':'Navigation'},blackboards:{'own-nav':{transit_routes:[route()]}},controlSources:{'own-nav':'Human'}});
it('projects only explicitly permitted known routes and never gives feed receivers authority',()=>{
 const source=state(),own=buildNavigationConsoleState(source,['own-nav']);
 expect(own.transit_routes).toEqual([route()]);expect(own.navigation_controllable).toBe(true);
 const feed=consoleReadState(source,{id:'receiver',systems:[],feeds:[{source:'own-nav',kind:'Navigation'}]});
 expect(buildNavigationConsoleState(feed,[])).toMatchObject({transit_routes:[route()],navigation_controllable:false});
 const isolated=consoleReadState(source,{id:'isolated',systems:[],feeds:[]});
 expect(buildNavigationConsoleState(isolated,[]).transit_routes).toEqual([]);
});
it('refuses malformed and ambiguous route readings including shared represented xz anchors',()=>{
 expect(buildTransitRoutes([route()])).toEqual([route()]);
 for(const rows of [[{...route(),point:[Infinity,0,0]}],[route(),route()],[route(),{...route(),id:'other',point:[10000,99,0]}]])expect(buildTransitRoutes(rows)).toEqual([]);
});
it('selects an exact current published route through existing owning paid waypoint action only',()=>{
 let view=buildNavigationConsoleState(state(),['own-nav']);const send=vi.fn();let serial=0;const registry=createNavigationActionRegistry({getState:()=>view,sendAction:send,actionFeedback:new ActionFeedbackLifecycle({correlation:()=> 'world-route-'+(++serial)})});
 const choose=()=>registry.activate(NAVIGATION_WORLD_ROUTE_ACTION_ID,{context:'navigation',detail:{id:'outer',point:[-1,0,-1]}});
 choose();expect(send).toHaveBeenCalledWith('set_navigation_waypoint',expect.objectContaining({control_system_id:'own-nav',x:10000,z:0}));
 send.mockClear();view={...view,navigation_controllable:false};choose();view={...view,navigation_controllable:true,navigation_auto:true};choose();view={...view,navigation_auto:false,transit_routes:[]};choose();expect(send).not.toHaveBeenCalled();
});
it('keeps route controls stable and disables withdrawn, delegated and detached actions',()=>{
 document.body.innerHTML='';window.activateSemanticAction=vi.fn();const widget=mountNavigationSharingControls(document);document.body.append(widget.node);
 let view=buildNavigationConsoleState(state(),['own-nav']);widget.update(view);const select=widget.node.querySelector('[data-world-routes]'),button=widget.node.querySelector('[data-world-route]');select.focus();const option=select.options[0];widget.update({...view,observation_tick:99});expect(select.options[0]).toBe(option);expect(document.activeElement).toBe(select);
 button.click();expect(window.activateSemanticAction).toHaveBeenCalledWith(NAVIGATION_WORLD_ROUTE_ACTION_ID,expect.objectContaining({detail:{id:'outer'}}));window.activateSemanticAction.mockClear();widget.update({...view,navigation_controllable:false});button.click();widget.update({...view,transit_routes:[]});button.click();widget.dispose();button.click();expect(window.activateSemanticAction).not.toHaveBeenCalled();
});
