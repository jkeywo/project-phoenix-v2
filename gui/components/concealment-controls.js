import {t} from '../strings.js';
import {CONCEALMENT_ENABLED_ACTION} from '../stations/concealment-actions.js';

/** Own performed work and operating emissions; no observer census or success. */
export function mountConcealmentControls(doc) {
  const node=doc.createElement('fieldset'),title=doc.createElement('legend'),status=doc.createElement('p');
  const work=doc.createElement('p'),signature=doc.createElement('p'),toggle=doc.createElement('button');
  node.dataset.concealment='';title.textContent=t('concealment.title');toggle.type='button';
  let owner=null,view=null,readonly=true;
  toggle.onclick=()=>{
    if(!readonly&&!toggle.disabled)doc.defaultView.activateSemanticAction?.(CONCEALMENT_ENABLED_ACTION.id,
      {context:'concealment',source:'control',detail:{owner,enabled:!view.enabled}});
  };
  node.append(title,status,work,signature,toggle);
  return {node,update(system,next,delegated=false) {
    owner=system;view=next;node.hidden=!next;readonly=delegated||!next?.controllable;
    toggle.disabled=readonly;toggle.hidden=readonly;
    if(!next)return;
    status.textContent=t(next.active?'concealment.active':next.enabled?'concealment.waiting':'concealment.off');
    work.textContent=t('concealment.work',{paid:(next.paid_per_mille/10).toFixed(1),perceptibility:(next.perceptibility_per_mille/10).toFixed(1)});
    signature.textContent=t('concealment.signature',{emission:next.operating_emission,cause:next.signature});
    toggle.textContent=t(next.enabled?'concealment.disable':'concealment.enable');
  },dispose(){node.remove();}};
}
