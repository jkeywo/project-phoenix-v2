/** @vitest-environment jsdom */
import {afterEach,expect,it,vi} from 'vitest';
import {trafficPayload} from '../../gui/comms-traffic.js';
import {mountCommsTrafficControls} from '../../gui/components/comms-traffic-controls.js';
import {registerTrafficActions,LISTEN_ORDER_ACTION,REPORT_ALERT_ACTION} from '../../gui/stations/comms-traffic-actions.js';
import {buildCommsConsoleState} from '../../gui/console-state.js';
import {makeCommsRender} from '../../gui/stations/comms-console.js';
import {createSemanticActionRegistry} from '../../gui/semantic-action-registry.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import {dispatchConsoleAction} from '../../gui/action-map.js';
const raw = () => ({channels:['public','fleet'],priority:['public','fleet'],simultaneous_channels:1,monitored:['public'],observed_tick:10,
  report_supported:true,last_report:null,controllable:false});
const view = (system='comms') => trafficPayload(raw(),system,true);
afterEach(() => {document.body.replaceChildren();delete window.activateSemanticAction;});

it('uses only selected own typed publication and current Human authority',() => {
  const state={blackboards:{radio:{traffic:raw()}},blackboardKinds:{radio:'Comms'},controlSources:{radio:'Human'}};
  expect(buildCommsConsoleState(state,['radio']).comms_traffic).toEqual(view('radio'));
  expect(buildCommsConsoleState(state,['other']).comms_traffic.controllable).toBe(false);
  expect(buildCommsConsoleState({...state,blackboards:{}}).comms_traffic).toBeUndefined();
  for(const bad of [{...raw(),priority:['public','public']},{...raw(),monitored:['fleet']},{...raw(),observed_tick:-1},
    {...raw(),simultaneous_channels:3},{...raw(),last_report:{semantic:'completed',emitted_tick:10}}, {...raw(),private_route:'secret'}])
    expect(trafficPayload(bad,'radio',true)).toBeNull();
});

it('rechecks exact owner and old order at semantic activation and emits closed correlated packets',() => {
  let current={comms_traffic:view()};const send=vi.fn();let serial=0;
  const registry=createSemanticActionRegistry({actionFeedback:new ActionFeedbackLifecycle({correlation:()=>`traffic-${++serial}`})});
  registerTrafficActions(registry,{getState:()=>current,sendAction:send});
  const choose=(detail={})=>registry.activate(LISTEN_ORDER_ACTION.id,{context:'comms-traffic',detail:{system:'comms',expected_priority:['public','fleet'],priority:['fleet','public'],...detail}});
  choose();expect(send).toHaveBeenLastCalledWith('set_comms_listening_order',expect.objectContaining({control_system_id:'comms',priority:['fleet','public']}));
  send.mockClear();choose({system:'foreign'});choose({expected_priority:['fleet','public']});choose({priority:['public','public']});
  current.comms_traffic.controllable=false;choose();expect(send).not.toHaveBeenCalled();
  current={comms_traffic:view()};registry.activate(REPORT_ALERT_ACTION.id,{context:'comms-traffic',detail:{system:'comms',private_alert:true}});
  expect(send).toHaveBeenCalledWith('report_comms_alert',expect.not.objectContaining({private_alert:true}));
  const wire=vi.fn();dispatchConsoleAction({action:'report_comms_alert',control_system_id:'comms',correlation:'proof',private_alert:true},wire);
  expect(wire).toHaveBeenCalledWith('ControlSystemCorrelated',{correlation:'proof',target:'comms',payload:{type:'ReportCommsAlert'}});
});

it('keeps controls stable while refusing held actions after priority/owner replacement or disposal',() => {
  const widget=mountCommsTrafficControls(document);document.body.append(widget.node);window.activateSemanticAction=vi.fn();
  widget.update(view());const first=widget.node.querySelectorAll('li button')[1];first.focus();widget.update(view());expect(document.activeElement).toBe(first);
  const down=widget.node.querySelectorAll('li button')[1];down.click();expect(window.activateSemanticAction).toHaveBeenCalledWith(LISTEN_ORDER_ACTION.id,expect.objectContaining({detail:{system:'comms',expected_priority:['public','fleet'],priority:['fleet','public']}}));
  window.activateSemanticAction.mockClear();down.dispatchEvent(new MouseEvent('mousedown'));
  widget.update({...view(),priority:['fleet','public'],monitored:['fleet']});down.dispatchEvent(new MouseEvent('click'));
  expect(window.activateSemanticAction).not.toHaveBeenCalled();
  const report=widget.node.querySelector('section > button');report.dispatchEvent(new MouseEvent('mousedown'));widget.update(view('new-radio'));report.click();expect(window.activateSemanticAction).not.toHaveBeenCalled();
  widget.dispose();report.dispatchEvent(new MouseEvent('click'));down.dispatchEvent(new MouseEvent('click'));widget.update(view());expect(window.activateSemanticAction).not.toHaveBeenCalled();
});

it('mounts outside the addressed inbox and withdraws the feature without an extra crew seat',() => {
  document.body.innerHTML='<div id="comms-traffic-host"></div><div id="thread"></div>';
  const render=makeCommsRender({ids:{threadPanel:'thread'}});render({messages:[],contacts:[],comms_traffic:view()},document);
  expect(document.getElementById('comms-traffic-host').hidden).toBe(false);expect(document.getElementById('thread').contains(document.querySelector('ol'))).toBe(false);
  render({messages:[],contacts:[]},document);expect(document.getElementById('comms-traffic-host').hidden).toBe(true);
});
