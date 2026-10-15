// @vitest-environment jsdom
import {afterEach,expect,it,vi} from 'vitest';
import {createSemanticActionRegistry} from '../../gui/semantic-action-registry.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
import {IDENTITY_ENABLED_ACTION,registerIdentityActions} from '../../gui/stations/identity-actions.js';
import {mountIdentityControls} from '../../gui/components/identity-controls.js';
import {mountSystemStatus} from '../../gui/components/ph-system-status.js';
import {dispatchConsoleAction} from '../../gui/action-map.js';
const view=()=>({enabled:true,active:true,callsign:'<img src=x onerror=attack()>',faction:null,controllable:true});
const row=()=>({system_id:'transponder',display_name:'Transponder',availability:'normal',reasons:[],delegated:false,identity:view()});
afterEach(()=>{delete window.activateSemanticAction;document.body.replaceChildren();});
it('admits only own current controls and keeps the exact correlated System target',()=>{
 const state={system_status:[row()]},send=vi.fn();let sequence=0;
 const registry=createSemanticActionRegistry({actionFeedback:new ActionFeedbackLifecycle({correlation:()=> `claim-${++sequence}`})});
 registerIdentityActions(registry,{getState:()=>state,sendAction:send});
 const activate=detail=>registry.activate(IDENTITY_ENABLED_ACTION.id,{context:'identity',detail});
 activate({owner:'transponder',enabled:false});
 const wire=vi.fn();for(const[action,fields]of send.mock.calls)dispatchConsoleAction({action,...fields},wire,vi.fn());
 expect(wire).toHaveBeenCalledWith('ControlSystemCorrelated',expect.objectContaining({target:'transponder',correlation:'claim-1',payload:{type:'SetIdentityEmitterEnabled',data:{enabled:false}}}));
 activate({owner:'foreign',enabled:true});activate({owner:'transponder',enabled:1});
 state.system_status[0].delegated=true;activate({owner:'transponder',enabled:true});
 state.system_status[0].delegated=false;state.system_status[0].identity.controllable=false;activate({owner:'transponder',enabled:true});
 expect(send).toHaveBeenCalledTimes(1);
});
it('renders a literal own claim and distinguishes paid, unfunded and disabled equipment',()=>{
 const widget=mountIdentityControls(document);document.body.append(widget.node);window.activateSemanticAction=vi.fn();
 widget.update('transponder',view());expect(widget.node.querySelector('img')).toBeNull();expect(widget.node.textContent).toContain(view().callsign);
 const paid=widget.node.querySelector('p').textContent;widget.node.querySelector('button').click();
 expect(window.activateSemanticAction).toHaveBeenCalledWith(IDENTITY_ENABLED_ACTION.id,expect.objectContaining({detail:{owner:'transponder',enabled:false}}));
 widget.update('transponder',{...view(),active:false});expect(widget.node.querySelector('p').textContent).not.toBe(paid);
 widget.update('transponder',{...view(),enabled:false,active:false});const disabled=widget.node.querySelector('p').textContent;expect(disabled).not.toBe(paid);
 widget.update('transponder',view(),true);expect(widget.node.querySelector('button').hidden).toBe(true);expect(widget.node.querySelector('button').disabled).toBe(true);
 widget.update('transponder',null);expect(widget.node.hidden).toBe(true);widget.dispose();
});
it('mounts actual equipment through common details and makes delegated controls read only',()=>{
 const widget=mountSystemStatus(document);widget.update([row()]);document.querySelector('[data-system-id="transponder"]').click();
 expect(document.querySelector('[data-identity-emitter]')).not.toBeNull();
 widget.update([{...row(),delegated:true}]);document.querySelector('[data-delegated-summary]').click();
 const button=document.querySelector('[data-identity-emitter] button');expect(button.hidden&&button.disabled).toBe(true);widget.dispose();
});
