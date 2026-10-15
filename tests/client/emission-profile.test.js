// @vitest-environment jsdom
import {afterEach,expect,it,vi} from 'vitest';
import {emissionIntentPayload,emissionAdvisoryPayload} from '../../gui/emission-profile.js';
import {registerEmissionProfileActions,EMISSION_INTENT_ACTION} from '../../gui/stations/emission-profile-actions.js';
import {mountEmissionAdvisory,mountEmissionProfileControls} from '../../gui/components/emission-profile-controls.js';
import {createSemanticActionRegistry} from '../../gui/semantic-action-registry.js';
import {ActionFeedbackLifecycle} from '../../gui/action-feedback.js';
afterEach(()=>{document.body.replaceChildren();delete window.activateSemanticAction;});
const view=(owner='orders')=>({owner,intent:'low_emission',controllable:true});
it('projects only the exact permitted owner and treats advice as measured information',()=>{
  expect(emissionIntentPayload({...view(),foreign:'hidden'},'orders',true)).toEqual(view());
  expect(emissionIntentPayload(view(),'elsewhere',true)).toBeNull();
  expect(emissionIntentPayload(view(),'orders',false).controllable).toBe(false);
  expect(emissionIntentPayload({...view(),intent:'invisible'},'orders',true)).toBeNull();
  const measured={observed_tick:7,rate:3,cause:'comms',advisory_rate:0.25,high:true};
  expect(emissionAdvisoryPayload({...measured,observer_uuid:'private'})).toEqual(measured);
  for(const bad of [{rate:NaN},{observed_tick:-1},{cause:'undetectable'},{advisory_rate:0}])expect(emissionAdvisoryPayload({...measured,...bad})).toBeNull();
});
it('rechecks current permitted Command choices and emits only its own closed intent',()=>{
  let views=[view()],sequence=0;const send=vi.fn(),registry=createSemanticActionRegistry({actionFeedback:new ActionFeedbackLifecycle({correlation:()=> `profile-${++sequence}`})});
  registerEmissionProfileActions(registry,{getViews:()=>views,sendAction:send});
  const activate=detail=>registry.activate(EMISSION_INTENT_ACTION.id,{context:'command-emission',detail});
  activate({owner:'orders',intent:'normal',limit:0});
  expect(send).toHaveBeenCalledWith('set_emission_intent',expect.objectContaining({control_system_id:'orders',intent:'normal',correlation:'profile-1'}));
  expect(send.mock.calls[0][1]).not.toHaveProperty('limit');
  activate({owner:'foreign',intent:'normal'});activate({owner:'orders',intent:'cloak'});
  views=[{...view(),controllable:false}];activate({owner:'orders',intent:'normal'});
  views=[view(),view()];activate({owner:'orders',intent:'normal'});
  views=[];activate({owner:'orders',intent:'normal'});expect(send).toHaveBeenCalledTimes(1);
});
it('shows readonly intent, invalidates retired owner buttons, and never disables machinery through advice',()=>{
  const widget=mountEmissionProfileControls(document);document.body.append(widget.node);window.activateSemanticAction=vi.fn();
  widget.update(view());const retired=widget.node.querySelector('button');retired.click();expect(window.activateSemanticAction).toHaveBeenCalledTimes(1);
  widget.update(view('other-orders'));retired.click();expect(window.activateSemanticAction).toHaveBeenCalledTimes(1);
  widget.update({...view('other-orders'),controllable:false});expect([...widget.node.querySelectorAll('button')].every(button=>button.disabled)).toBe(true);
  expect(widget.node.querySelector('p').textContent).not.toBe('');widget.dispose();retired.click();expect(window.activateSemanticAction).toHaveBeenCalledTimes(1);
  const machinery=document.createElement('button'),advice=mountEmissionAdvisory(document);document.body.append(machinery,advice.node);
  advice.update({high:true,rate:3,cause:'comms'});expect(advice.node.hidden).toBe(false);expect(machinery.disabled).toBe(false);
  advice.update(null);expect(advice.node.hidden).toBe(true);advice.dispose();
});
