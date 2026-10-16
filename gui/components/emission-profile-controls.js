import {t} from '../strings.js';
import {EMISSION_INTENT_ACTION} from '../stations/emission-profile-actions.js';
import {emissionIntents} from '../emission-profile.js';

export function mountEmissionProfileControls(doc) {
  const node=doc.createElement('fieldset'),title=doc.createElement('legend'),status=doc.createElement('p');
  node.dataset.emissionProfile='';title.textContent=t('emission.intent');node.append(title,status);
  let active=null,disposed=false,buttons=[];
  function rebuild(owner){
    for(const[,button]of buttons)button.disabled=true;
    buttons=emissionIntents.map(intent=>{
    const button=doc.createElement('button');button.type='button';button.textContent=t('emission.'+intent);
    button.onclick=()=>{if(!disposed&&!button.disabled&&active?.controllable&&active.owner===owner)doc.defaultView.activateSemanticAction?.(EMISSION_INTENT_ACTION.id,{context:'command-emission',source:'control',detail:{owner,intent}});};
    return [intent,button];
    });
    node.replaceChildren(title,status,...buttons.map(([,button])=>button));
  }
  return {node,update(view){
    if(disposed)return;if(active?.owner!==view?.owner)rebuild(view?.owner);active=view;node.hidden=!view;status.textContent=view?t('emission.'+view.intent):'';
    for(const[intent,button]of buttons){button.disabled=!view?.controllable;button.hidden=!view?.controllable;button.setAttribute('aria-pressed',String(view?.intent===intent));}
  },dispose(){disposed=true;active=null;for(const[,button]of buttons)button.disabled=true;node.remove();}};
}
/** Advice is a separate label; never pass a machinery control to this helper. */
export function mountEmissionAdvisory(doc) {
  const node=doc.createElement('p');node.dataset.emissionAdvisory='';node.setAttribute('role','status');
  return {node,update(view){node.hidden=!view?.high;node.textContent=view?.high?t('emission.high',{rate:view.rate,cause:t('emission.cause.'+view.cause)}):'';},dispose(){node.remove();}};
}
