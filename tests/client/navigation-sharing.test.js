/** @vitest-environment jsdom */
import {beforeEach,describe,it,expect,vi} from 'vitest';
import {mountNavigationSharingControls} from '../../gui/components/navigation-sharing-controls.js';
import {createNavigationActionRegistry,NAVIGATION_SHARED_WAYPOINT_ACTION_ID} from '../../gui/stations/navigation-actions.js';
import {createRadarSharingActionRegistry,RADAR_SHARING_ACTION_ID} from '../../gui/stations/radar-sharing-actions.js';
import {ACTION_MAP} from '../../gui/action-map.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
const owner=()=>({owner:'custom-navigation',role:'navigation',share_own_faction:false,receive:true,received_samples:1,controllable:true,can_operate:true});
const sample=(id='original',position=[5,8])=>({provenance:{sample_id:id,origin:{source:'radar-origin-0000000000000001',system:'nav-a',contact:'waypoint'},observed_tick:3,received_tick:7},published_tick:3,position,kind:{kind:'waypoint'}});
const state=()=>({radar_sharing:[owner()],received_chart:[sample()],navigation_auto:false});
beforeEach(()=>{document.body.innerHTML='';window.activateSemanticAction=vi.fn();});
describe('actual Navigation sharing',()=>{
  it('applies only an explicitly chosen held fixed point through the exact owned normal command',()=>{
    let view=state(),next=0;const sent=vi.fn(),registry=createNavigationActionRegistry({getState:()=>view,sendAction:sent,actionFeedback:new ActionFeedbackLifecycle({correlation:()=>`nav-share-${++next}`})});
    const options={context:'navigation',detail:{owner:'custom-navigation',sample_id:'original'}};
    registry.activate(NAVIGATION_SHARED_WAYPOINT_ACTION_ID,options);expect(sent).toHaveBeenCalledWith('set_navigation_waypoint',expect.objectContaining({control_system_id:'custom-navigation',x:5,z:8}));
    const wire=vi.fn();ACTION_MAP.set_navigation_waypoint(sent.mock.calls[0][1],wire);expect(wire).toHaveBeenCalledWith('ControlSystemCorrelated',expect.objectContaining({target:'custom-navigation',payload:{type:'SetNavigationWaypoint',data:{x:5,z:8}}}));
    sent.mockClear();view={...view,radar_sharing:[{...owner(),controllable:false}]};registry.activate(NAVIGATION_SHARED_WAYPOINT_ACTION_ID,options);
    view={...state(),navigation_auto:true};registry.activate(NAVIGATION_SHARED_WAYPOINT_ACTION_ID,options);
    view={...state(),received_chart:[sample('new-choice',[20,30])]};registry.activate(NAVIGATION_SHARED_WAYPOINT_ACTION_ID,options);expect(sent).not.toHaveBeenCalled();
  });
  it('keeps received point controls stable across clock snapshots and rejects detached old choices',()=>{
    const widget=mountNavigationSharingControls(document);document.body.append(widget.node);widget.update(state());
    const button=widget.node.querySelector('button');button.focus();widget.update({...state(),observation_tick:100});expect(widget.node.querySelector('button')).toBe(button);expect(document.activeElement).toBe(button);
    button.click();expect(window.activateSemanticAction).toHaveBeenCalledWith(NAVIGATION_SHARED_WAYPOINT_ACTION_ID,expect.objectContaining({detail:{sample_id:'original',owner:'custom-navigation'}}));
    window.activateSemanticAction.mockClear();widget.update({...state(),received_chart:[sample('new-choice')]});button.click();expect(window.activateSemanticAction).not.toHaveBeenCalled();
    const outer=vi.fn();document.addEventListener('keydown',outer);widget.node.querySelector('button').dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true}));expect(outer).not.toHaveBeenCalled();document.removeEventListener('keydown',outer);
    widget.dispose();
  });
  it('reuses the faction share command for Navigation without adding a controller',()=>{
    const sent=vi.fn(),registry=createRadarSharingActionRegistry({getState:state,sendAction:sent,actionFeedback:new ActionFeedbackLifecycle({correlation:()=> 'nav-share-toggle'})});
    registry.activate(RADAR_SHARING_ACTION_ID,{context:'navigation',detail:{owner:'custom-navigation',share_own_faction:true,receive:false}});
    expect(sent).toHaveBeenCalledWith('set_radar_sharing',expect.objectContaining({control_system_id:'custom-navigation',share_own_faction:true,receive:false}));
  });
});
