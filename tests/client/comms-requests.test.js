// @vitest-environment jsdom
import {afterEach, describe, expect, it, vi} from 'vitest';
import '../../gui/components/ph-comms-text-composer.js';
import '../../gui/components/ph-comms-current-message.js';
import {createCommsActionRegistry, COMMS_SEND_REQUEST_ACTION_ID, COMMS_MARK_READ_ACTION_ID, COMMS_RESPOND_ACTION_ID} from '../../gui/stations/comms-actions.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import {dispatchConsoleAction} from '../../gui/action-map.js';
import {makeCommsRender} from '../../gui/stations/comms-console.js';
import {requestStatus, visibleCommsMessages} from '../../gui/comms-requests.js';
import {activeThreadMessageFrom} from '../../gui/comms-state.js';
import {t} from '../../gui/strings.js';
const received = () => ({id:'heard',sender_uuid:'peer',sender_name:'Peer',thread_id:'thread',body:'Report status.',is_read:false,
  sender_in_range:false,responses:[{text:'Report',available:true},{text:'Decline',available:true}],
  transmission:{channel:'public',emitted_tick:10,received_tick:12,content:{kind:'request',request:'report_status'}},
  request_progress:{facts:{read_tick:null,decision:null,report:null},dismissed:false,last_returned:null}});
afterEach(()=>{document.body.replaceChildren();delete window.activateSemanticAction;});
describe('closed generic Comms requests',()=>{
  it('normal exact semantic actions emit correlated typed packets and reject unknown or delegated choices',()=>{
    let view={channels:[{id:'public'}],contacts:[{uuid:'peer'}],messages:[received()],comms_auto:false};
    const packets=[];const sendAction=vi.fn((action,data)=>dispatchConsoleAction({action,...data},(type,data)=>packets.push({type,data}),()=>{}));
    const registry=createCommsActionRegistry({getState:()=>view,sendAction,actionFeedback:new ActionFeedbackLifecycle()});
    const detail={channel:'public',audience:{kind:'contact',target_uuid:'peer'},request:'report_status'};
    expect(registry.activate(COMMS_SEND_REQUEST_ACTION_ID,{context:'comms',source:'control',detail}).handled).toBe(true);
    expect(packets[0]).toEqual({type:'ControlSystemCorrelated',data:{correlation:expect.any(String),target:'comms',payload:{type:'SendCommsRequest',data:detail}}});
    expect(registry.activate(COMMS_MARK_READ_ACTION_ID,{context:'comms',source:'control',detail:{message_id:'heard'}}).handled).toBe(true);
    expect(packets[1].data.payload).toEqual({type:'MarkCommsRead',data:{message_id:'heard'}});
    for(const invalid of [{...detail,channel:'secret'},{...detail,audience:{kind:'contact',target_uuid:'hidden'}}])
      expect(registry.activate(COMMS_SEND_REQUEST_ACTION_ID,{context:'comms',source:'control',detail:invalid}).handled).toBe(false);
    view={...view,comms_auto:true};
    expect(registry.activate(COMMS_MARK_READ_ACTION_ID,{context:'comms',source:'control',detail:{message_id:'heard'}}).handled).toBe(false);
    expect(registry.activate(COMMS_SEND_REQUEST_ACTION_ID,{context:'comms',source:'control',detail}).handled).toBe(false);
    expect(packets).toHaveLength(2);
  });
  it('delegated structured responses are disabled and every semantic input refuses them',()=>{
    const message=received();
    document.body.innerHTML='<ph-comms-current-message></ph-comms-current-message>';
    const el=document.body.firstElementChild;window.activateSemanticAction=vi.fn();
    el.state={thread:message,messages:[message],read_only:true};
    for(const button of el.shadowRoot.querySelectorAll('.resp-btn')) {
      expect(button.disabled).toBe(true);button.click();
    }
    expect(window.activateSemanticAction).not.toHaveBeenCalled();
    const sendAction=vi.fn();
    const registry=createCommsActionRegistry({getState:()=>({messages:[message],comms_auto:true}),sendAction});
    for(const detail of [undefined,{message_id:message.id,response_index:0},{message_id:message.id,response_index:1}])
      expect(registry.activate(COMMS_RESPOND_ACTION_ID,{context:'comms',source:'control',detail}).handled).toBe(false);
    expect(sendAction).not.toHaveBeenCalled();
    el.state={thread:message,messages:[message],read_only:false};
    expect([...el.shadowRoot.querySelectorAll('.resp-btn')].every(button=>!button.disabled)).toBe(true);
  });
  it('composer sends ReportStatus without consuming or modifying a literal text draft',()=>{
    document.body.innerHTML='<ph-comms-text-composer></ph-comms-text-composer>';
    const el=document.body.firstElementChild;el.state={channels:[{id:'public'}],contacts:[],messages:[],comms_auto:false};
    el.shadowRoot.getElementById('body').value='Keep this text';
    window.activateSemanticAction=vi.fn(()=>({handled:true}));
    el.shadowRoot.getElementById('request').click();
    expect(window.activateSemanticAction).toHaveBeenCalledWith(COMMS_SEND_REQUEST_ACTION_ID,expect.objectContaining({detail:expect.objectContaining({channel:'public',request:'report_status'})}));
    expect(el.shadowRoot.getElementById('body').value).toBe('Keep this text');
  });
  it('received requests stay locally answerable without any current remote-listener oracle',()=>{
    const message=received();
    expect(activeThreadMessageFrom([message],'thread')).toBe(message);
    const unavailableText={...message,transmission:{...message.transmission,content:{kind:'text'}}};
    expect(activeThreadMessageFrom([unavailableText],'thread')).toBeNull();
  });
  it('renders original reported clocks and actual explicit read without presenting internal delivery frontier',()=>{
    document.body.innerHTML='<ph-comms-current-message></ph-comms-current-message>';
    const el=document.body.firstElementChild;const message=received();window.activateSemanticAction=vi.fn();
    el.state={thread:message,messages:[message]};el.shadowRoot.getElementById('read-request').click();
    expect(window.activateSemanticAction).toHaveBeenCalledWith(COMMS_MARK_READ_ACTION_ID,{source:'control',detail:{message_id:'heard'}});
    const outcome={...message,id:'returned',is_read:true,responses:[],transmission:{emitted_tick:90,received_tick:120,content:{kind:'outcome',outcome:{request:{heard_tick:12},facts:{read_tick:32,decision:{choice:'report',tick:40},report:{system:'comms',available:false,observed_tick:40}}}}},request_progress:{dismissed:false,last_returned:'completed'}};
    el.state={thread:outcome,messages:[outcome]};
    const expected=requestStatus(outcome);
    expect(expected).toContain(t('comms.request.heard_at',{tick:12}));expect(expected).toContain(t('comms.request.read_at',{tick:32}));
    expect(expected).toContain(t('comms.request.unavailable_at',{tick:40}));
    expect(el.shadowRoot.querySelector('.request-status').textContent).toBe(expected);
    expect(expected).not.toMatch(/120|90|accepted|last_returned/);
    expect(el.shadowRoot.getElementById('read-request').hidden).toBe(true);
  });
  it('retains cleared history for same Console filter and new outcomes remain independently visible',()=>{
    const old={...received(),id:'old',thread_id:'old-thread',is_read:true,request_progress:{dismissed:true}};
    const fresh={...received(),id:'new',thread_id:'new-thread'};
    expect(visibleCommsMessages([old,fresh])).toEqual([fresh]);
    document.body.innerHTML='<ph-comms-text-composer id="comms-text-composer"></ph-comms-text-composer><ph-unupgraded id="list"></ph-unupgraded><ph-unupgraded id="message"></ph-unupgraded>';
    const render=makeCommsRender({ids:{hailList:'list',currentMessage:'message'}});
    render({channels:[{id:'public'}],messages:[old,fresh],contacts:[]},document);
    expect(document.getElementById('list').state.threads.map(row=>row.thread_id)).toEqual(['new-thread']);
    const checkbox=document.getElementById('comms-text-composer').shadowRoot.getElementById('cleared');
    checkbox.checked=true;checkbox.dispatchEvent(new Event('change'));
    expect(document.getElementById('list').state.threads.map(row=>row.thread_id)).toEqual(['new-thread','old-thread']);
    checkbox.checked=false;checkbox.dispatchEvent(new Event('change'));
    expect(document.getElementById('list').state.threads.map(row=>row.thread_id)).toEqual(['new-thread']);
  });
});

describe('actual own planned request choices',()=>{
  const row=()=>({id:'htn/maintenance/request',released_tick:10,recipient_uuid:'peer',channel:'public',request:'report_status'});
  it('uses the ordinary correlated payload and rejects mismatched, withdrawn or delegated contexts',()=>{
    let view={channels:[{id:'public'},{id:'fleet'}],contacts:[{uuid:'peer'}],messages:[],planned_requests:[row()],comms_auto:false};
    const packets=[];const sendAction=vi.fn((action,data)=>dispatchConsoleAction({action,...data},(type,data)=>packets.push({type,data}),()=>{}));
    const registry=createCommsActionRegistry({getState:()=>view,sendAction,actionFeedback:new ActionFeedbackLifecycle()});
    const detail={channel:'public',audience:{kind:'contact',target_uuid:'peer'},request:'report_status',planned_request:row().id};
    expect(registry.activate(COMMS_SEND_REQUEST_ACTION_ID,{context:'comms',source:'control',detail}).handled).toBe(true);
    expect(packets[0].data.payload).toEqual({type:'SendCommsRequest',data:detail});
    for(const invalid of [{...detail,planned_request:'other-instance'},{...detail,channel:'fleet'},{...detail,audience:{kind:'all'}},{...detail,planned_request:'x'.repeat(2049)}])
      expect(registry.activate(COMMS_SEND_REQUEST_ACTION_ID,{context:'comms',source:'control',detail:invalid}).handled).toBe(false);
    for(const replacement of [{...view,contacts:[],messages:[received()]},{...view,planned_requests:[]},{...view,comms_auto:true}]){
      view=replacement;expect(registry.activate(COMMS_SEND_REQUEST_ACTION_ID,{context:'comms',source:'control',detail}).handled).toBe(false);
    }
    expect(packets).toHaveLength(1);
  });
  it('shows known target labels safely and preserves draft/focus when sending a planned choice',()=>{
    document.body.innerHTML='<ph-comms-text-composer></ph-comms-text-composer>';
    const el=document.body.firstElementChild,state={channels:[{id:'public'}],contacts:[{uuid:'peer',name:'<b>Beacon</b>'}],messages:[],planned_requests:[row()],comms_auto:false};
    el.state=state;const select=el.shadowRoot.getElementById('planned'),body=el.shadowRoot.getElementById('body');
    expect(select.options[0].textContent).toContain('<b>Beacon</b>');expect(el.shadowRoot.querySelector('b')).toBeNull();
    body.value='Keep literal text';select.focus();const option=select.options[0];el.state={...state};
    expect(select.options[0]).toBe(option);expect(el.shadowRoot.activeElement).toBe(select);
    window.activateSemanticAction=vi.fn(()=>({handled:true}));el.shadowRoot.getElementById('send-planned').click();
    expect(window.activateSemanticAction).toHaveBeenCalledWith(COMMS_SEND_REQUEST_ACTION_ID,{source:'control',detail:{channel:'public',audience:{kind:'contact',target_uuid:'peer'},request:'report_status',planned_request:row().id}});
    expect(body.value).toBe('Keep literal text');
    el.state={...state,comms_auto:true};el.shadowRoot.getElementById('send-planned').click();expect(window.activateSemanticAction).toHaveBeenCalledTimes(1);
    el.state={...state,contacts:[]};expect(el.shadowRoot.getElementById('planned-controls').hidden).toBe(true);
  });
});


describe('ordered Formation coordination surface',()=>{
  it('uses the same request action with a closed finite slot and known recipient',()=>{
    const packets=[];
    const view={channels:[{id:'public'}],contacts:[{uuid:'peer'}],messages:[],comms_auto:false};
    const registry=createCommsActionRegistry({getState:()=>view,actionFeedback:new ActionFeedbackLifecycle(),sendAction:(action,data)=>dispatchConsoleAction({action,...data},(type,data)=>packets.push({type,data}),()=>{})});
    const detail={channel:'public',audience:{kind:'contact',target_uuid:'peer'},request:{form_up:{slot:[-40,80]}}};
    expect(registry.activate(COMMS_SEND_REQUEST_ACTION_ID,{context:'comms',source:'control',detail}).handled).toBe(true);
    expect(packets[0].data.payload).toEqual({type:'SendCommsRequest',data:detail});
    for(const invalid of [{...detail,request:{form_up:{slot:[0,NaN]}}},{...detail,request:{form_up:{slot:[0,1000001]}}},
      {...detail,request:{form_up:{slot:[0,1],leader:'private'}}},{...detail,audience:{kind:'all'}},
      {...detail,audience:{kind:'contact',target_uuid:'hidden'}}])
      expect(registry.activate(COMMS_SEND_REQUEST_ACTION_ID,{context:'comms',source:'control',detail:invalid}).handled).toBe(false);
    expect(packets).toHaveLength(1);
  });
  it('distinguishes coordination acceptance and shows the actual typed slot',()=>{
    const message=received();message.transmission.content.request={form_up:{slot:[-40,80]}};
    message.request_progress.facts={read_tick:14,decision:{choice:'accept',tick:15},report:null};
    const text=requestStatus(message);
    expect(text).toContain(t('comms.request.accepted_at',{tick:15}));
    expect(text).toContain(t('comms.request.formation_slot',{x:-40,z:80}));
    expect(text).not.toContain(t('comms.request.available_at',{tick:15}));
  });
  it('has explicit Contact-only Formation controls without clearing the text draft',()=>{
    document.body.innerHTML='<ph-comms-text-composer></ph-comms-text-composer>';
    const el=document.body.firstElementChild;window.activateSemanticAction=vi.fn();
    el.state={channels:[{id:'public'}],contacts:[{uuid:'peer',name:'Peer'}],messages:[],comms_auto:false};
    const root=el.shadowRoot;root.querySelector('#body').value='Unsent draft';
    expect(root.querySelector('#formation-controls').disabled).toBe(true);
    root.querySelector('#audience').value='contact:peer';root.querySelector('#audience').dispatchEvent(new Event('change'));
    root.querySelector('#slot-x').value='-70';root.querySelector('#slot-z').value='20';
    root.querySelector('#form-up').click();
    expect(window.activateSemanticAction).toHaveBeenCalledWith(COMMS_SEND_REQUEST_ACTION_ID,{source:'control',detail:{channel:'public',audience:{kind:'contact',target_uuid:'peer'},request:{form_up:{slot:[-70,20]}}}});
    expect(root.querySelector('#body').value).toBe('Unsent draft');
  });
});
