/** @vitest-environment jsdom */
import {beforeEach,describe,it,expect,vi} from 'vitest';
import {mountRadarSharingControls} from '../../gui/components/radar-sharing-controls.js';
import {createRadarSharingActionRegistry,RADAR_SHARING_ACTION_ID} from '../../gui/stations/radar-sharing-actions.js';
import {ACTION_MAP} from '../../gui/action-map.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
const reading = (owner='sensor-one') => ({owner,role:'sensor_radar',share_own_faction:false,receive:true,received_samples:2,controllable:true,can_operate:true});
beforeEach(()=>{document.body.innerHTML='';window.activateSemanticAction=vi.fn();});
describe('deliberate scoped radar sharing',()=>{
  it('sends exact-owner correlated intent and refuses foreign, stale and unavailable rows',()=>{
    let state={radar_sharing:[reading(),{...reading('foreign'),controllable:false}]};
    let next=0;const sent=vi.fn(),registry=createRadarSharingActionRegistry({getState:()=>state,sendAction:sent,
      actionFeedback:new ActionFeedbackLifecycle({correlation:()=> 'share-'+(++next)})});
    const options=owner=>({context:'sensors',detail:{owner,share_own_faction:true,receive:false}});
    registry.activate(RADAR_SHARING_ACTION_ID,options('foreign'));expect(sent).not.toHaveBeenCalled();
    registry.activate(RADAR_SHARING_ACTION_ID,options('sensor-one'));
    expect(sent).toHaveBeenCalledWith('set_radar_sharing',expect.objectContaining({control_system_id:'sensor-one',share_own_faction:true,receive:false}));
    const wire=vi.fn();ACTION_MAP.set_radar_sharing({...sent.mock.calls[0][1],correlation:'share-1'},wire);
    expect(wire).toHaveBeenCalledWith('ControlSystemCorrelated',{correlation:'share-1',target:'sensor-one',payload:{type:'SetRadarSharing',data:{share_own_faction:true,receive:false}}});
    sent.mockClear();state={radar_sharing:[{...reading(),can_operate:false}]};registry.activate(RADAR_SHARING_ACTION_ID,options('sensor-one'));
    state={radar_sharing:[]};registry.activate(RADAR_SHARING_ACTION_ID,options('sensor-one'));expect(sent).not.toHaveBeenCalled();
  });
  it('keeps native controls and focus through progress snapshots and revokes held actuation',()=>{
    const widget=mountRadarSharingControls(document);document.body.append(widget.node);widget.update([reading()]);
    const share=widget.node.querySelector('input');share.focus();
    widget.update([{...reading(),received_samples:3}]);expect(widget.node.querySelector('input')).toBe(share);expect(document.activeElement).toBe(share);
    share.checked=true;share.dispatchEvent(new Event('change'));expect(window.activateSemanticAction).toHaveBeenCalledWith(RADAR_SHARING_ACTION_ID,expect.objectContaining({detail:{owner:'sensor-one',share_own_faction:true,receive:true}}));
    const outer=vi.fn();widget.node.addEventListener('keydown',outer);share.dispatchEvent(new KeyboardEvent('keydown',{key:' ',bubbles:true}));expect(outer).not.toHaveBeenCalled();
    window.activateSemanticAction.mockClear();widget.update([{...reading(),controllable:false}]);
    expect(share.disabled).toBe(true);expect(share.parentElement.hidden).toBe(true);share.dispatchEvent(new Event('change'));expect(window.activateSemanticAction).not.toHaveBeenCalled();
    widget.update([]);expect(widget.node.hidden).toBe(true);widget.dispose();
  });
});
