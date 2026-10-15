import {t} from '../strings.js';
import {IDENTITY_ENABLED_ACTION} from '../stations/identity-actions.js';
/** Own actual paid signal only. No observer attribution or successful deception. */
export function mountIdentityControls(doc) {
  const node=doc.createElement('fieldset'),title=doc.createElement('legend'),status=doc.createElement('p'),callsign=doc.createElement('p'),toggle=doc.createElement('button');
  node.dataset.identityEmitter='';title.textContent=t('identity.title');toggle.type='button';
  let owner=null,view=null,readonly=true;
  toggle.onclick=()=>{if(!readonly&&!toggle.disabled)doc.defaultView.activateSemanticAction?.(IDENTITY_ENABLED_ACTION.id,{context:'identity',source:'control',detail:{owner,enabled:!view.enabled}});};
  node.append(title,status,callsign,toggle);
  return {node,update(system,next,delegated=false) {
    owner=system;view=next;node.hidden=!next;readonly=delegated||!next?.controllable;toggle.disabled=readonly;toggle.hidden=readonly;
    if(!next)return;
    status.textContent=t(next.active?'identity.active':next.enabled?'identity.waiting':'identity.off');
    callsign.textContent=t('identity.callsign',{callsign:next.callsign});
    toggle.textContent=t(next.enabled?'identity.disable':'identity.enable');
  },dispose(){node.remove();}};
}
