import { SOUND_CUES_JSON, validateSoundCatalog } from './sound-cues.js';
import { renderAudioSettingsPanel } from './audio-settings-panel.js';
import { t, wireText } from './strings.js';

/** Existing-surface local preview. No transport or live action capability. */
export function mountSoundAudition({root,audio,win=root.ownerDocument.defaultView,
  load=()=>win.fetch(SOUND_CUES_JSON).then(response=>{if(!response.ok) throw new Error('catalog');return response.json();}),
  assets=null}={}) {
  const doc=root.ownerDocument, panel=doc.createElement('fieldset');panel.className='sound-audition';
  const make=(tag,id)=>{const node=doc.createElement(tag);node.textContent=t(`sound_cues.${id}`);return node;};
  panel.append(make('legend','title'),make('p','scope'));
  const select=doc.createElement('select');select.dataset.soundCue='';
  const label=make('label','definition');label.append(select);panel.append(label);
  const status=doc.createElement('p');status.setAttribute('role','status');status.dataset.soundStatus='';
  const equivalent=doc.createElement('p');equivalent.dataset.soundEquivalent='';equivalent.hidden=true;
  equivalent.setAttribute('role','status');equivalent.setAttribute('aria-live','polite');equivalent.setAttribute('aria-atomic','true');
  const findings=doc.createElement('p');findings.setAttribute('role','alert');
  const preview=make('button','preview'),stop=make('button','stop'),reload=make('button','refresh');
  preview.type=stop.type=reload.type='button';preview.dataset.soundPreview='';stop.dataset.soundStop='';
  panel.append(preview,stop,reload,status,equivalent,findings);
  const settings=doc.createElement('details');settings.append(make('summary','settings'));panel.append(settings);
  const disposeSettings=audio?renderAudioSettingsPanel(doc,settings,audio):()=>{};
  root.append(panel);
  let cues=[],allowed=assets,pending=false,disposed=false,serial=0,timer=null,active=false,held=false,loading=false;
  let message='ready';
  const chosen=()=>cues.find(cue=>cue.id===select.value);
  function paint() {
    preview.disabled=held||loading||pending||!chosen();select.disabled=held||loading;reload.disabled=held||loading;
    stop.disabled=!active&&!pending;
    const state=audio?.state();
    status.textContent=[t(`sound_cues.${message}`),t(`settings.audio.output_${state?.status||'unavailable'}`),
      state?.auditionAvailable?'':t('sound_cues.unavailable')].filter(Boolean).join(' ');
  }
  function cancel() {
    serial++;pending=false;active=false;loading=false;if(timer!==null)win.clearTimeout(timer);timer=null;
    audio?.stopAudition();equivalent.hidden=true;equivalent.textContent='';message='ready';paint();
  }
  function showEquivalent(cue) {
    const value=cue.equivalent;equivalent.hidden=!value;
    equivalent.textContent=value?[wireText(value.meaning),t('sound_cues.source',{source:wireText(value.source)}),
      t(`sound_cues.urgency_${value.urgency}`),value.bearing==null?'':t('sound_cues.bearing',{value:value.bearing}),
      value.elevation==null?'':t('sound_cues.elevation',{value:value.elevation})].filter(Boolean).join(' · '):'';
  }
  async function refresh() {
    cancel();loading=true;paint();const generation=serial;
    try {
      const catalog=await load();if(disposed||generation!==serial)return;
      allowed=catalog.assets;
      const checked=validateSoundCatalog(catalog,assets||undefined),current=select.value;
      cues=checked.cues;select.replaceChildren(...cues.map(cue=>{const option=doc.createElement('option');option.value=cue.id;option.textContent=wireText(cue.label);return option;}));
      if(current&&cues.some(cue=>cue.id===current))select.value=current;
      findings.textContent=checked.findings.map(finding=>t(`sound_cues.invalid_${finding.code}`,{id:finding.id})).join(' ');
    } catch(_) {if(generation===serial){cues=[];select.replaceChildren();findings.textContent=t('sound_cues.invalid_catalog');}}
    finally {if(generation===serial){loading=false;paint();}}
  }
  preview.addEventListener('click',async()=>{
    const cue=chosen();if(!cue||held||loading||pending)return;
    cancel();const generation=serial;pending=true;active=true;showEquivalent(cue);message='preparing';paint();
    let played=false;try{played=!!await audio?.audition(cue,allowed);}catch(_){/* visible status below */}
    if(disposed||generation!==serial)return;
    pending=false;message=played?'playing':'silent';paint();
    timer=win.setTimeout(()=>{if(generation===serial)cancel();},2100);
  });
  stop.addEventListener('click',cancel);select.addEventListener('change',cancel);reload.addEventListener('click',()=>void refresh());
  const hide=()=>{if(doc.hidden)cancel();};doc.addEventListener('visibilitychange',hide);win.addEventListener('pagehide',cancel);
  const unsubscribe=audio?.subscribe(paint);const ready=refresh();
  return {ready,refresh,stop:cancel,setHeld(value){held=value===true;if(held)cancel();panel.hidden=held;paint();},dispose(){disposed=true;cancel();unsubscribe?.();disposeSettings();doc.removeEventListener('visibilitychange',hide);win.removeEventListener('pagehide',cancel);panel.remove();}};
}
