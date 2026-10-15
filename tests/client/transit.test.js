/** @vitest-environment jsdom */
import {afterEach,expect,it,vi} from 'vitest';
import {buildTransitDrives} from '../../gui/transit.js';
import {buildHelmConsoleState,buildConsoleView} from '../../gui/console-state.js';
import {createHelmActionRegistry} from '../../gui/stations/helm-actions.js';
import {dispatchConsoleAction} from '../../gui/action-map.js';
import {makeHelmRender} from '../../gui/stations/helm-console.js';
import '../../gui/components/ph-transit-drives.js';

const dto=()=>({owner:'actual-drive',phase:'idle',preparation_fraction:0,fuel_required:2,can_begin:true,can_cancel:false,waiting:null,request:null,
  course:{kind:'local',point:[100,0,30],course_generation:7},actual_arrival:null,last_step_tick:12});
const entries=value=>[{systemId:'actual-drive',data:value}];
const options={systemIds:['actual-drive'],controlSources:{'actual-drive':'Human'}};
afterEach(()=>{document.body.replaceChildren();delete window.activateSemanticAction;});
it('uses only matching typed owner data and keeps read-only/unavailable phases honest',()=>{
  const value=dto();
  expect(buildTransitDrives(entries(value),options)[0]).toMatchObject({system_id:'actual-drive',controllable:true,can_begin:true,can_cancel:false});
  expect(buildTransitDrives(entries(value),{...options,systemIds:[]})[0]).toMatchObject({controllable:false,can_begin:false});
  expect(buildTransitDrives(entries(value),{...options,controlSources:{'actual-drive':'Ai'}})[0].can_begin).toBe(false);
  for(const edit of [row=>row.owner='private-owner',row=>row.phase='remote',row=>row.fuel_required=-1,row=>row.course.point[1]=Infinity,row=>row.actual_arrival=[0,0,0]]){
    const bad=structuredClone(value);edit(bad);expect(buildTransitDrives(entries(bad),options)).toEqual([]);
  }
});
it('normal Console projection accepts only the exact explicit typed Transit feed',()=>{
  const definition={id:'bridge-helm',home_station:'helm',systems:['helm-thrust'],feeds:[]};
  const state={consoleDefinitions:[definition],systemConsoleFamilies:{'helm-thrust':'helm','actual-drive':'helm'},controlSources:{'actual-drive':'Human'},
    blackboardKinds:{'actual-drive':'Transit'},blackboards:{'actual-drive':dto()},
    systemKinds:{'actual-drive':'transit-drive','helm-thrust':'helm_thrust'}};
  expect(buildConsoleView('bridge-helm',state).transit_drives).toBeUndefined();
  definition.feeds=[{source:'actual-drive',kind:'Transit'}];
  expect(buildConsoleView('bridge-helm',state).transit_drives).toMatchObject([{system_id:'actual-drive',controllable:false}]);
  definition.feeds=[{source:'actual-drive',kind:'Helm'}];
  expect(buildConsoleView('bridge-helm',state).transit_drives).toBeUndefined();
  expect(buildHelmConsoleState({asteroids:[{uuid:'private-body',position:[100,0,30]}]},[]).transit_drives).toBeUndefined();
});
it('manual Begin and Cancel use exact current owner with ordinary correlated commands',()=>{
  let value=dto(),view={transit_drives:buildTransitDrives(entries(value),options)};
  const wire=vi.fn();let serial=0;
  const registry=createHelmActionRegistry({getState:()=>view,sendAction:(action,data)=>dispatchConsoleAction({action,...data},wire),
    actionFeedback:{press:()=>({correlation:'jump-'+(++serial),inputMs:serial}),pending(){},cancel(){},settle(){}}});
  expect(registry.activate('helm.transit-begin',{context:'helm',detail:{systemId:'actual-drive',point:[999,999,999]}}).handled).toBe(true);
  expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated',expect.objectContaining({target:'actual-drive',payload:{type:'BeginTransitToWaypoint',data:{request:null}}}));
  value={...value,phase:'preparing',preparation_fraction:.5,can_begin:false,can_cancel:true};view={transit_drives:buildTransitDrives(entries(value),options)};
  expect(registry.activate('helm.transit-begin',{context:'helm',detail:{systemId:'actual-drive'}}).handled).toBe(false);
  expect(registry.activate('helm.transit-cancel',{context:'helm',detail:{systemId:'actual-drive'}}).handled).toBe(true);
  expect(wire).toHaveBeenLastCalledWith('ControlSystemCorrelated',expect.objectContaining({target:'actual-drive',payload:{type:'CancelTransit'}}));
  view={transit_drives:buildTransitDrives(entries({...value,phase:'committed',preparation_fraction:1,can_cancel:false}),options)};
  expect(registry.activate('helm.transit-cancel',{context:'helm',detail:{systemId:'actual-drive'}}).handled).toBe(false);
  expect(registry.activate('helm.transit-begin',{context:'helm',detail:{systemId:'private-owner'}}).handled).toBe(false);
});
it('real Helm renderer keeps stable drive controls across preparation and arrival',()=>{
  document.body.innerHTML='<ph-transit-drives id="transit-drives"></ph-transit-drives>';
  const activate=vi.fn();window.activateSemanticAction=activate;
  const node=document.getElementById('transit-drives'),render=makeHelmRender({ids:{}}),value=dto();
  render({transit_drives:buildTransitDrives(entries(value),options)},document);
  const root=node.shadowRoot,begin=root.querySelector('[data-kind="begin"]');begin.click();
  expect(activate).toHaveBeenCalledWith('helm.transit-begin',expect.objectContaining({detail:{systemId:'actual-drive'}}));
  begin.focus();render({transit_drives:buildTransitDrives(entries({...value,phase:'preparing',can_begin:false,can_cancel:true,preparation_fraction:.5,waiting:'unfunded'}),options)},document);
  expect(root.querySelector('[data-kind="begin"]')).toBe(begin);expect(begin.disabled).toBe(true);expect(root.textContent).toContain('50%');
  render({transit_drives:buildTransitDrives(entries({...value,phase:'arrived',preparation_fraction:1,actual_arrival:[101,0,31]}),options)},document);
  expect(root.textContent).toContain('101.0, 0.0, 31.0');
  render({},document);expect(node.hidden).toBe(true);
});
