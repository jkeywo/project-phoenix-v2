/** @vitest-environment jsdom */
import {it,expect,vi} from 'vitest';
import {formationNavigation} from '../../gui/formation.js';
import {buildNavigationConsoleState} from '../../gui/console-state.js';
import {consoleReadState} from '../../gui/console-topology.js';
import {createNavigationActionRegistry,NAVIGATION_FORMATION_ACTION_ID} from '../../gui/stations/navigation-actions.js';
import {mountFormationCourseControls} from '../../gui/components/formation-course-controls.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import {ACTION_MAP} from '../../gui/action-map.js';
const request=()=>({message_id:'received/2',origin_id:'issued/2',source_name:'Known leader',sequence:2,slot:[-40,80],accepted_tick:10});
const view=()=>({suggestions:[request()],leaders:[{contact:'own/contact',name:'Known leader'}],course:null});
const source=()=>({blackboardKinds:{'own-nav':'Navigation'},blackboards:{'own-nav':{formation:view()}},controlSources:{'own-nav':'Human'}});
it('requires finite dated public data, distinct identities and byte-bounded names',()=>{
 expect(formationNavigation(view())).toEqual(view());
 const selected={owner:'own-nav',request:request(),leader:'own/contact',selected_tick:11,paused:false,measurement:{position:[100,200],velocity:[2,3],observed_tick:11}};
 expect(formationNavigation({...view(),course:selected}).course).toEqual(selected);
 for(const bad of [{...view(),suggestions:[request(),request()]},{...view(),leaders:[{contact:'own/contact'},{contact:'own/contact'}]},
   {...view(),suggestions:[{...request(),slot:[1e6+1,0]}]},{...view(),suggestions:[{...request(),source_name:'é'.repeat(257)}]},
   {...view(),course:{...selected,selected_tick:9}},{...view(),course:{...selected,measurement:{...selected.measurement,velocity:[Infinity,0]}}}])expect(formationNavigation(bad)).toBeNull();
});
it('shares only explicit Navigation information and never confers authority on a feed receiver',()=>{
 const raw=source(),own=buildNavigationConsoleState(raw,['own-nav']);
 expect(own).toMatchObject({formation:view(),navigation_system_id:'own-nav',navigation_controllable:true});
 const feed=consoleReadState(raw,{id:'reader',systems:[],feeds:[{source:'own-nav',kind:'Navigation'}]});
 expect(buildNavigationConsoleState(feed,[])).toMatchObject({formation:view(),navigation_controllable:false});
 const isolated=consoleReadState(raw,{id:'isolated',systems:[],feeds:[]});expect(buildNavigationConsoleState(isolated,[]).formation).toBeNull();
});
it('rechecks exact current request and earned alias while keeping coordinates and sender UUID off the command',()=>{
 let current=buildNavigationConsoleState(source(),['own-nav']);const send=vi.fn();let sequence=0;
 const actions=createNavigationActionRegistry({getState:()=>current,sendAction:send,actionFeedback:new ActionFeedbackLifecycle({correlation:()=>`formation-${++sequence}`})});
 const choose=(detail={request_id:'received/2',leader:'own/contact',x:999,target_uuid:'private'})=>actions.activate(NAVIGATION_FORMATION_ACTION_ID,{context:'navigation',detail});
 expect(choose()).toMatchObject({handled:true});expect(send).toHaveBeenCalledWith('select_formation_course',expect.objectContaining({control_system_id:'own-nav',request_id:'received/2',leader:'own/contact'}));
 const packet=send.mock.calls[0][1];expect(packet).not.toHaveProperty('x');expect(packet).not.toHaveProperty('target_uuid');
 send.mockClear();choose({request_id:'old',leader:'own/contact'});choose({request_id:'received/2',leader:'private-source-uuid'});
 current={...current,navigation_controllable:false};choose();current={...current,navigation_controllable:true,navigation_auto:true};choose();
 current={...current,navigation_auto:false,formation:{...view(),suggestions:[]}};choose();expect(send).not.toHaveBeenCalled();
 const wire=vi.fn();ACTION_MAP.select_formation_course({...packet,correlation:'choice',target_uuid:'private',position:[999,999]},wire);
 expect(wire).toHaveBeenCalledWith('ControlSystemCorrelated',{correlation:'choice',target:'own-nav',payload:{type:'SelectFormationCourse',data:{request_id:'received/2',leader:'own/contact'}}});
});
it('keeps focused choices stable and disables stale, delegated and disposed controls',()=>{
 document.body.innerHTML='';window.activateSemanticAction=vi.fn();const widget=mountFormationCourseControls(document);document.body.append(widget.node);
 const current={navigation_controllable:true,navigation_auto:false,formation:view()};widget.update(current);
 const requests=widget.node.querySelector('[data-formation-requests]'),button=widget.node.querySelector('[data-formation-select]');requests.focus();const option=requests.options[0];
 widget.update({...current,observation_tick:99});expect(requests.options[0]).toBe(option);expect(document.activeElement).toBe(requests);
 button.click();expect(window.activateSemanticAction).toHaveBeenCalledWith(NAVIGATION_FORMATION_ACTION_ID,expect.objectContaining({detail:{request_id:'received/2',leader:'own/contact'}}));
 window.activateSemanticAction.mockClear();widget.update({...current,navigation_controllable:false});button.click();widget.update({...current,formation:{...view(),leaders:[]}});button.click();widget.dispose();button.click();expect(window.activateSemanticAction).not.toHaveBeenCalled();
});
