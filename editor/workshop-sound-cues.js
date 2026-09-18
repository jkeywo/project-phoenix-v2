import { parse } from 'smol-toml';
import { createPrivateAudio } from '../gui/private-audio.js';
import { mountSoundAudition } from '../gui/sound-audition-panel.js';
import { SOUND_CUES_JSON, SOUND_CUES_PATH } from '../gui/sound-cues.js';

/** Authoring-only source preview. Native Workshop has no assigned endpoint;
 * its validation/equivalent remains usable and sound stays explicitly absent. */
export function mountWorkshopSoundCues({root,win,draft,readAudio,saveAudio,native=false,resolveAsset=async()=>null,attach=true}) {
  // The catalog arrives asynchronously, so the dockable node has to exist now:
  // a stable host the audition panel mounts into once the fetch settles.
  const host=root.ownerDocument.createElement('div');host.className='workshop-sound';host.id='workshop-sound';
  // A docked panel is placed by the renderer; attaching here too would strand the
  // node in `root` whenever the stored layout has the panel closed.
  if(attach)root.append(host);
  let previous=null,revision=null,held=false,visible=true,refreshOwed=false;
  // An audition the operator cannot see is stopped, not left running from a
  // detached node on the two-second timer alone.
  const suppressed=()=>held||!visible;
  const apply=()=>{panel?.setHeld(suppressed());
    if(!suppressed()&&refreshOwed&&panel){refreshOwed=false;void panel.refresh();}};
  const catalog=Promise.resolve().then(()=>win.fetch(SOUND_CUES_JSON)).then(response=>{if(!response.ok)throw new Error('catalog');return response.json();});
  const audio=createPrivateAudio({root:win,requireNativeProvider:native,allowAudition:true,
    read:readAudio,save:saveAudio,isEnabled:()=>false,
    fetchAudio:async file=>{
      const current=draft();
      if(current?.paths().includes(file) && current.isBinary(file)) {
        const bytes=current.bytes(file);return {ok:true,arrayBuffer:async()=>bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength)};
      }
      const bytes=await resolveAsset(file);
      return bytes?{ok:true,arrayBuffer:async()=>bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength)}:{ok:false};
    }});
  let panel=null,disposed=false;
  const ready=catalog.then(base=>{
    if(disposed)return;
    panel=mountSoundAudition({root:host,audio,win,assets:base.assets,load:()=>{
      const source=draft()?.read(SOUND_CUES_PATH);
      return source?parse(source):base;
    }});refreshOwed=suppressed();apply();return panel.ready;
  }).catch(()=>{
    if(!disposed) {panel=mountSoundAudition({root:host,audio,win,load:()=>Promise.reject(new Error('catalog'))});apply();}
  });
  return {ready,refresh({held:nextHeld=false}={}) {
    const current=draft(),next=current?.sourceRevision;
    if(current!==previous || next===undefined || next!==revision) {previous=current;revision=next;refreshOwed=true;}
    held=nextHeld;apply();
  },setVisible(value){if(visible===value)return;visible=value;apply();},
  node:host,dispose(){disposed=true;panel?.dispose();audio.dispose();host.remove();}};
}
