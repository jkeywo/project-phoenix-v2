// @vitest-environment jsdom
import {afterEach,expect,it,vi} from 'vitest';
import {WorkshopDocument} from '../../editor/workshop-document.js';
import {createStoreZip} from '../../editor/mod-pack-export.js';
import {mountWorkshopEmissionProfile} from '../../gui/workshop-emission-profile-panel.js';
const path='assets/entities/reference.toml';
const source='name="Reference"\n[[console]]\nid="orders"\n[[system]]\nid="commands"\nkind="command"\nconsole="orders"\n';
const make=text=>new WorkshopDocument(createStoreZip([{path:'scenarios.toml',text:'[pack]\nformat=1\nid="emission"\nname="Emission"\nversion="1"\n'},{path,text}]));
afterEach(()=>document.body.replaceChildren());
it('offers actual Command ownership and routes Preview/Test through the existing callbacks',async()=>{
  let draft=make(source);const preview=vi.fn(),test=vi.fn();
  const panel=mountWorkshopEmissionProfile({root:document.body,draft:()=>draft,runtime:{dependencies:async()=>({base_files:{},packs:[]})},preview,test});
  await vi.waitFor(()=>expect(document.querySelector('#workshop-emission-owner').value).toBe('commands'));
  document.querySelector('#workshop-emission-preview').click();document.querySelector('#workshop-emission-test').click();
  expect(preview).toHaveBeenCalledWith(path);expect(test).toHaveBeenCalledWith(path);
  // A different source owner with the same revision/path cannot reuse options.
  draft=make(source.replace('kind="command"','kind="comms"'));panel.update();
  expect(document.querySelector('#workshop-emission-save').disabled).toBe(true);
  const retired=document.querySelector('#workshop-emission-preview');panel.dispose();retired.click();panel.update();expect(preview).toHaveBeenCalledTimes(1);
});
it('disposal refuses a late accepted edit instead of writing a retired draft',async()=>{
  const draft=make(source),before=draft.read(path);let finish;
  const runtime={dependencies:async()=>({base_files:{},packs:[]}),validate:async()=>({accepted:true}),compose:vi.fn(()=>new Promise(resolve=>finish=resolve))};
  const panel=mountWorkshopEmissionProfile({root:document.body,draft:()=>draft,runtime});
  await vi.waitFor(()=>expect(document.querySelector('#workshop-emission-save').disabled).toBe(false));
  document.querySelector('#workshop-emission-materialise').checked=true;document.querySelector('#workshop-emission-save').click();
  await vi.waitFor(()=>expect(runtime.compose).toHaveBeenCalledTimes(1));
  panel.dispose();finish(before+'\n[system.config.emission_profile]\ninitial="normal"\nadvisory_rate=0.25\n');
  await new Promise(resolve=>setTimeout(resolve,0));expect(draft.read(path)).toBe(before);
});
