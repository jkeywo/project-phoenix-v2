import { test, expect } from '@playwright/test';

const BASE = `<!doctype html><html><head><link rel="stylesheet" href="/client/gui/audio-settings.css"></head><body>
<main id="audio"></main><button id="next">Current transmission</button>
<ph-comms-hail-list></ph-comms-hail-list><ph-comms-current-message></ph-comms-current-message>
<script type="module">
import '/client/gui/strings-boot.js';
import '/client/gui/components/ph-comms-hail-list.js';
import '/client/gui/components/ph-comms-current-message.js';
import { ClientSimState } from '/client/gui/sim-state.js';
import { createStationPrivateAlerts } from '/client/gui/private-alerts.js';
import { createPrivateAudio } from '/client/gui/private-audio.js';
import { renderAudioSettingsPanel } from '/client/gui/audio-settings-panel.js';
window.audio=createPrivateAudio({root:window});
window.alerts=createStationPrivateAlerts({audio}); window.state=new ClientSimState();
state.stationSystems={captain:['command'], auxiliary:['actual-comms']};
state.systemKinds={'actual-comms':'comms'}; state.controlSources={'actual-comms':'Human'};
window.uiState={phase:'InProgress',players:[{token:'me',station:'captain',connected:true}]};
window.messages=[]; window.generation=1;
window.sample=(rows,host='captain')=>{
  window.messages=rows;
  const changes=state.apply({type:'BlackboardUpdate',data:{presentation_generation:generation,updates:[['actual-comms',{kind:'Comms',data:{host_station:host,messages:rows}}]]}});
  alerts.update({state,uiState,token:'me',connected:true,changes});
  document.querySelector('ph-comms-hail-list').state={messages:rows};
  document.querySelector('ph-comms-current-message').state={thread:rows.at(-1),messages:rows};
};
window.message=(id,extra={})=>({id,thread_id:id,sender_name:'Axiom',body:'Hold position and await instructions.',priority:'Urgent',is_read:false,selected_response:null,responses:[],...extra});
sample([]); document.getElementById('next').onclick=()=>sample([...messages,message('live-'+messages.length)]);
renderAudioSettingsPanel(document,document.getElementById('audio'),audio);
</script></body></html>`;

test('hosted Comms alerts use current source edges and retain Urgent/Critical text while muted or restored', { tag: '@core' }, async ({ page }) => {
  const errors=[]; page.on('pageerror', e=>errors.push(String(e)));
  await page.route('**/client/private-alerts-probe', route=>route.fulfill({contentType:'text/html',body:BASE}));
  await page.goto('/client/private-alerts-probe');
  await expect.poll(()=>page.evaluate(()=>window.audio?.state().ready.length)).toBe(7);
  await page.locator('[data-audio-enable]').click();
  await page.locator('#next').click();
  await expect.poll(()=>page.evaluate(()=>audio.debug().outputPeak)).toBeGreaterThan(0.00001);
  await expect(page.locator('ph-comms-hail-list .priority-text')).toHaveText('[Urgent]');
  await expect.poll(()=>page.evaluate(()=>audio.state().active.length)).toBe(0);
  await page.evaluate(()=>sample(messages)); expect(await page.evaluate(()=>audio.state().active.length)).toBe(0);
  await page.locator('[data-audio-bus="alerts"] button').click();
  await page.evaluate(()=>sample([message('critical',{priority:'Critical',is_read:true,sender_in_range:false})]));
  expect(await page.evaluate(()=>audio.state().active.length)).toBe(0);
  await expect(page.locator('ph-comms-current-message #priority-cue')).toContainText('CRITICAL');
  await page.locator('[data-audio-bus="alerts"] button').click();
  await page.evaluate(()=>sample(messages)); expect(await page.evaluate(()=>audio.state().active.length)).toBe(0);
  await page.evaluate(()=>{generation++;sample([message('restored')]);});
  expect(await page.evaluate(()=>audio.state().active.length)).toBe(0);
  await page.evaluate(()=>sample([message('not-our-host')],'tactical'));
  expect(await page.evaluate(()=>audio.state().active.length)).toBe(0);
  await page.setViewportSize({width:390,height:844}); await page.emulateMedia({forcedColors:'active'});
  await page.addStyleTag({content:'html{font-size:200%} body{margin:8px} ph-comms-hail-list,ph-comms-current-message{display:block;max-width:100%}'});
  expect(await page.evaluate(()=>{
    const nodes = root => [...root.querySelectorAll('*')].flatMap(el=>[el,...(el.shadowRoot?nodes(el.shadowRoot):[])]);
    return {overflow:document.documentElement.scrollWidth>innerWidth,
      elements:nodes(document).filter(el=>el.getBoundingClientRect().right>innerWidth)
        .map(el=>({tag:el.tagName,cls:el.className,width:el.getBoundingClientRect().width,right:el.getBoundingClientRect().right}))};
  })).toEqual({overflow:false,elements:[]});
  expect(errors).toEqual([]);
});

const GM = `<!doctype html><html><body><main id="audio"></main><script type="module">
import '/client/gui/strings-boot.js';
import {mountGmWorkspace} from '/client/gui/gm-workspace.js';
import {renderAudioSettingsPanel} from '/client/gui/audio-settings-panel.js';
window.__phoenixGmPage=true;window.__hostLocalGm=()=>({id:'gm',connected:true});
window.workspace=mountGmWorkspace({win:window});window.audio=window.__privateAudio;
window.attention=(rows,generation=1)=>workspace.handlers.gm_attention({occurrences:rows,presentation_generation:generation});
window.health=(rows,generation=1)=>workspace.handlers.gm_health({peers:[],alerts:rows,presentation_generation:generation});
window.occurrence=(id)=>({id,band:'urgent',category:'idle_npc',first_seen_tick:1,age_ms:0,reason:{id:'server.gm.attention.reason.pending_comms',params:{sender:'Axiom',ship:'Alpha'}},target:{}});
window.failure=(id)=>({id,kind:'ship_peer_lost',severity:'disconnected',reason:{id:'server.gm.health.reason.recovery_failed',params:{tick:'30'}}});
attention([]);health([]);renderAudioSettingsPanel(document,document.getElementById('audio'),audio);
</script></body></html>`;

test('private GM attention and health reach actual output once and resume silently at a new generation', {tag:'@core'}, async ({page})=>{
  const errors=[];page.on('pageerror', e=>errors.push(String(e)));
  await page.route('**/client/private-gm-alerts-probe',route=>route.fulfill({contentType:'text/html',body:GM}));
  await page.goto('/client/private-gm-alerts-probe');
  await expect.poll(()=>page.evaluate(()=>window.audio?.state().ready.length)).toBe(7);
  await page.evaluate(()=>{attention([]);health([]);});
  await page.locator('[data-audio-enable]').click();
  await page.evaluate(()=>attention([occurrence('new#1')]));
  await expect.poll(()=>page.evaluate(()=>audio.debug().outputPeak)).toBeGreaterThan(0.00001);
  await expect.poll(()=>page.evaluate(()=>audio.state().active.length)).toBe(0);
  await page.evaluate(()=>attention([occurrence('new#1')]));expect(await page.evaluate(()=>audio.state().active.length)).toBe(0);
  await page.evaluate(()=>health([failure('peer#1')]));
  await expect.poll(()=>page.evaluate(()=>audio.debug().outputPeak)).toBeGreaterThan(0.00001);
  await expect.poll(()=>page.evaluate(()=>audio.state().active.length)).toBe(0);
  await page.evaluate(()=>{attention([occurrence('restore#1')],2);health([failure('restore#1')],2);});
  expect(await page.evaluate(()=>audio.state().active.length)).toBe(0);
  expect(errors).toEqual([]);
});
