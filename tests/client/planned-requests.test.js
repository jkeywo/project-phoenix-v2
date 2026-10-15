/** @vitest-environment jsdom */
import {afterEach,expect,it,vi} from 'vitest';
import {plannedRequests,plannedRepairDispatch,plannedDeliveryDispatch} from '../../gui/planned-requests.js';
import {buildRepairConsoleState,buildLogisticsConsoleState} from '../../gui/console-state.js';
import {createEngineeringActionRegistry,REPAIR_DISPATCH_ACTION_ID} from '../../gui/stations/engineering-actions.js';
import {LOGISTICS_ACTION_IDS} from '../../gui/stations/logistics-actions.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import {dispatchConsoleAction} from '../../gui/action-map.js';
import {renderPlannedRequests} from '../../gui/components/planned-requests.js';
import '../../gui/components/ph-repair-teams.js';
import '../../gui/components/ph-logistics-controls.js';
const work={kind:'compartment',compartment:'engineering'};
const repair=()=>({id:'htn/maintenance/root/restore',released_tick:20,team:1,request:{type:'Work',data:work},target:work,goal:'structure'});
const delivery=()=>({id:'htn/maintenance/root/resupply',released_tick:40,source:'hold',destination:'magazine',commodity:'torpedoes',amount:4});
const stores=[{id:'hold',role:'main',resource:'torpedoes'},{id:'magazine',role:'local',resource:'torpedoes'}];
const repairView=()=>({system_id:'actual-repair',teams:[{id:1,status:'idle'}],planned_requests:[repair()],planned_controllable:true});
const deliveryView=()=>({system_id:'actual-logistics',stores,max_batch:4,planned_requests:[delivery()],planned_controllable:true});
afterEach(()=>{document.body.replaceChildren();delete window.activateSemanticAction;});
it('closed instructions preserve original release ticks and never acquire physical status',()=>{
  const source=repair(),rows=plannedRequests([source],'repair');expect(rows).toEqual([source]);source.target.compartment='changed';expect(rows[0].target.compartment).toBe('engineering');
  for(const bad of [{...repair(),team:256},{...repair(),goal:'automatic'},{...repair(),goal:'full'},{...repair(),current_hp:0},{...repair(),request:{type:'External'}},{...repair(),released_tick:NaN}])expect(plannedRequests([bad],'repair')).toEqual([]);
  expect(plannedRequests([delivery(),delivery()],'delivery')).toEqual([]);
  expect(plannedRequests([{...delivery(),amount:0}],'delivery')).toEqual([]);
});
it('actual typed publication projection keeps explicit feed knowledge separate from ownership',()=>{
  const state={blackboardKinds:{'actual-repair':'Repair','actual-logistics':'Logistics'},blackboards:{'actual-repair':{planned_requests:[repair()],teams:[]},'actual-logistics':{planned_requests:[delivery()]}},controlSources:{'actual-repair':'Human','actual-logistics':'Human'}};
  expect(buildRepairConsoleState(state,[]).planned_controllable).toBe(false);
  expect(buildRepairConsoleState(state,['actual-repair']).planned_controllable).toBe(true);
  expect(buildLogisticsConsoleState(state,[]).planned_controllable).toBe(false);
  state.controlSources['actual-logistics']='Ai';expect(buildLogisticsConsoleState(state,['actual-logistics']).planned_controllable).toBe(false);
  delete state.blackboards['actual-repair'].planned_requests;expect(buildRepairConsoleState(state,['actual-repair']).planned_requests).toBeUndefined();
});
it('normal correlated owner actions use current exact mission payloads and reject revoked/forged requests',()=>{
  const repair=repairView(),logistics=deliveryView();const state={system_ids:['actual-repair','actual-logistics'],system_families:{'actual-repair':'repair','actual-logistics':'logistics'},systems:{'actual-repair':repair,'actual-logistics':logistics}};
  let serial=0;const send=vi.fn(),registry=createEngineeringActionRegistry({getState:()=>state,sendAction:send,actionFeedback:new ActionFeedbackLifecycle({correlation:()=>`plan-${++serial}`})});
  expect(registry.activate(REPAIR_DISPATCH_ACTION_ID,{context:'engineering',detail:{planned_request:repair.planned_requests[0].id,target:'foreign',team_idx:0}}).handled).toBe(true);
  expect(registry.activate(LOGISTICS_ACTION_IDS.dispatch,{context:'engineering',detail:{planned_request:logistics.planned_requests[0].id,amount:999}}).handled).toBe(true);
  const wire=vi.fn();for(const [action,data] of send.mock.calls)dispatchConsoleAction({action,...data},wire);
  expect(wire.mock.calls[0][1]).toMatchObject({target:'actual-repair',payload:{type:'DispatchRepairTeam',data:{team_idx:1,target:{type:'Work',data:work}}}});
  expect(wire.mock.calls[1][1]).toMatchObject({target:'actual-logistics',payload:{type:'DispatchDelivery',data:{source:'hold',destination:'magazine',amount:4}}});
  repair.planned_controllable=false;logistics.planned_requests=[];
  expect(registry.activate(REPAIR_DISPATCH_ACTION_ID,{context:'engineering',detail:{planned_request:'htn/maintenance/root/restore'}}).handled).toBe(false);
  expect(registry.activate(LOGISTICS_ACTION_IDS.dispatch,{context:'engineering',detail:{planned_request:'htn/maintenance/root/resupply'}}).handled).toBe(false);
  expect(send).toHaveBeenCalledTimes(2);
});
it('current own availability limits voluntary request actions without reading hidden work',()=>{
  const repair=repairView();repair.teams[0].status='travelling';expect(plannedRepairDispatch(repair,repair.planned_requests[0].id)).toBeNull();
  const delivery=deliveryView();delivery.stores=[stores[0]];expect(plannedDeliveryDispatch(delivery,delivery.planned_requests[0].id)).toBeNull();
});
it('stable instruction nodes retain focus and held controls cannot activate after revocation',()=>{
  const container=document.createElement('section');document.body.append(container);const activate=vi.fn();const view=repairView();
  renderPlannedRequests(container,view,activate);const button=container.querySelector('button');button.focus();
  renderPlannedRequests(container,structuredClone(view),activate);expect(container.querySelector('button')).toBe(button);expect(document.activeElement).toBe(button);
  button.click();expect(activate).toHaveBeenCalledTimes(1);
  renderPlannedRequests(container,{planned_requests:[],planned_controllable:false},activate);button.click();expect(activate).toHaveBeenCalledTimes(1);expect(container.hidden).toBe(true);
});
it('real Repair and Logistics widgets render voluntary mission instructions without dispatching on paint',()=>{
  const activate=vi.fn();window.activateSemanticAction=activate;
  const repair=document.createElement('ph-repair-teams'),delivery=document.createElement('ph-logistics-controls');document.body.append(repair,delivery);
  repair.state={...repairView(),auto:false};delivery.state={...deliveryView(),enabled:true,deliveries:[]};
  expect(activate).not.toHaveBeenCalled();expect(repair.shadowRoot.querySelector('#planned-requests').textContent).toContain('Requested work');
  repair.shadowRoot.querySelector('#planned-requests button').click();expect(activate).toHaveBeenLastCalledWith(REPAIR_DISPATCH_ACTION_ID,expect.objectContaining({detail:{planned_request:'htn/maintenance/root/restore'}}));
  const offered=[...delivery.shadowRoot.querySelectorAll('button')].find(button=>button.textContent==='Undertake');expect(offered).toBeTruthy();offered.click();expect(activate).toHaveBeenLastCalledWith(LOGISTICS_ACTION_IDS.dispatch,expect.objectContaining({detail:{planned_request:'htn/maintenance/root/resupply'}}));
});
