import { test, expect } from '@playwright/test';

test('shared GM Sound control reaches actual room MP3 output and current equivalent without replay', {tag:'@core'}, async({page}) => {
  const errors=[];page.on('pageerror', error=>errors.push(String(error)));
  await page.route('**/client/live-sound-probe', route=>route.fulfill({contentType:'text/html',body:`<!doctype html>
    <html><head><base href='/client/'><link rel='stylesheet' href='gui/audio-settings.css'>
    <link rel='stylesheet' href='gui/audio-live-equivalents.css'><style>body{margin:8px}fieldset{min-width:0;max-width:35rem}label{display:block}select,input,textarea{max-width:90%}</style></head>
    <body><section id='gm-mission-panel'></section><script type='module'>
    import '/client/gui/strings-boot.js';
    import {t} from '/client/gui/strings.js';
    import {createGmPresentationPanel} from '/client/gui/gm-presentation-panel.js';
    import {createHostAudio} from '/client/gui/host-audio.js';
    import {createAudioLiveEquivalents} from '/client/gui/audio-live-equivalents.js';
    const catalog=await fetch('assets/audio/sound-cues.json').then(r=>r.json());
    const definitions=catalog.cues.filter(c=>c.audience==='viewscreen');
    const equivalents=createAudioLiveEquivalents(document);
    window.audio=createHostAudio({doc:document,storage:localStorage,onEquivalent:cue=>equivalents.update(cue)});
    audio.audioLifecycle(JSON.stringify({generation:1,running:true,suspended:false}));
    audio.audioConfig(JSON.stringify({authored_sounds:definitions}));await audio.enable();
    window.records=[];let serial=0;
    const projection={entities:[{entity_id:'alpha',kind:'player_ship',name:'Horizon'}],presentation_sounds:definitions.map(c=>c.id)};
    window.panel=createGmPresentationPanel({t,getOperator:()=>({id:'gm'}),submit:record=>{
      records.push(record);window.packet=JSON.stringify({kind:'authored',generation:1,occurrence:++serial,definition:definitions.find(c=>c.id===record.cue.sound.id)});
      audio.audioCue(packet);setTimeout(()=>panel.update({...projection,presentation_results:[{operator_id:record.operator_id,correlation:record.correlation,outcome:'applied'}]}),0);return true;
    }});panel.update(projection);
    </script></body></html>`}));
  await page.goto('/client/live-sound-probe');
  const sound=page.locator('#gm-presentation-sound');await expect(sound).toBeEnabled();
  await sound.selectOption('weapons');
  await expect.poll(()=>page.evaluate(()=>audio.state().ready.includes('authored_weapons'))).toBe(true);
  const play=page.getByRole('button',{name:'Play sound',exact:false});await play.focus();await page.keyboard.press('Enter');
  await expect.poll(()=>page.evaluate(()=>audio.debug().outputPeak)).toBeGreaterThan(0.00001);
  await expect(page.locator('[data-audio-equivalent=authored]')).toContainText('45');
  expect(await page.evaluate(()=>records[0].cue)).toEqual({sound:{id:'weapons',source:null}});
  await page.evaluate(()=>{audio.setBus('master',{muted:true});audio.audioCue(packet);audio.setBus('master',{muted:false});audio.audioCue(packet);});
  await expect.poll(()=>page.evaluate(()=>audio.debug().outputPeak)).toBeLessThan(0.00001);
  await page.evaluate(()=>audio.audioLifecycle(JSON.stringify({generation:2,running:true,suspended:true})));
  await page.evaluate(()=>{audio.audioLifecycle(JSON.stringify({generation:3,running:true,suspended:false}));audio.audioCue(packet);});
  expect(await page.evaluate(()=>audio.state().active.filter(v=>v.id.startsWith('authored_')))).toEqual([]);
  await page.setViewportSize({width:390,height:844});await page.emulateMedia({forcedColors:'active'});
  await page.addStyleTag({content:'body{font-size:200%}'});await play.focus();
  expect(await play.evaluate(node=>document.activeElement===node)).toBe(true);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  expect(errors).toEqual([]);
});
