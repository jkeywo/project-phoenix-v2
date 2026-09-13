import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import path from 'node:path';

const root = path.resolve(__dirname, '../..');
const boot = readFileSync(path.join(root,'src/native_host/audio/private_boot.js'),'utf8');
const storage = readFileSync(path.join(root,'src/native_host/panes/operator_storage.js'),'utf8');
const refusedFixture = readFileSync(path.join(root,'tests/fixtures/native-private-refused.json'),'utf8').trim();
const testFixture = readFileSync(path.join(root,'tests/fixtures/native-private-test.json'),'utf8').trim();
const SETUP = `<script>${storage}\n${boot}\n
window.operatorRecords=[];
PhoenixInstallNativeOperatorStorage(record=>{operatorRecords.push(record);return true;});
__phoenixOperatorReply({operation:'load',status:'ok',profile:null});
window.publishAudio = (status='playing',generation=1) => __phoenixPrivateAudioApply(JSON.stringify({status,generation,revision:0,test:'idle',surface:'fixture',outputs:['output:Private headset'],categories:['alerts','interface'],detail: status==='playing'?'':'settings.audio.selected_missing'}));
window.takeAudio = () => {const raw=__phoenixPrivateAudioDrain();if(!raw)return '';const record=JSON.parse(raw);record.generation=0;if(record.cue)record.cue.at_ms=0;return JSON.stringify(record);};
</script>`;

const STATION = `<!doctype html><html><head><link rel="stylesheet" href="/client/gui/audio-settings.css">${SETUP}</head><body>
<main></main><iframe id="station"></iframe><script type="module">
import '/client/gui/strings-boot.js';
import {createPrivateAudio,privateFeedbackReceiver} from '/client/gui/private-audio.js';
import {renderAudioSettingsPanel} from '/client/gui/audio-settings-panel.js';
window.audio=createPrivateAudio({root:window});window.sent=[];
const frame=document.getElementById('station');
window.addEventListener('message',event=>{if(event.source===frame.contentWindow&&event.data?.type==='console_action')sent.push(JSON.parse(event.data.payload));});
window.__privateActionFeedback=privateFeedbackReceiver({getAudio:()=>audio,currentSource:()=>frame.contentWindow});
renderAudioSettingsPanel(document,document.querySelector('main'),audio);
frame.srcdoc='<script type="module">import {initConsole} from "/client/gui/console-core.js";window.__sendAction=json=>parent.sent.push(JSON.parse(json));window.runtime=initConsole({name:"captain",render:()=>{}});window.__updateConsole("captain",JSON.stringify({red_alert:false,red_alert_auto:false}));<\\/script>';
</script></body></html>`;

const GM = `<!doctype html><html><head><link rel="stylesheet" href="/client/gui/audio-settings.css">${SETUP}</head><body>
<section id="gm-mission-panel"></section><main></main><script type="module">
import {mountNativeGmWorkspace} from '/client/gui/native-gm-workspace.js';
import {renderAudioSettingsPanel} from '/client/gui/audio-settings-panel.js';
window.sent=[];let receiver;
window.workspace=mountNativeGmWorkspace({bridge:{getOperator:()=>({id:'native-gm',connected:true}),submitAction:request=>{sent.push(request);return true;},subscribe:fn=>{receiver=fn;return()=>{};},setReady:()=>true,forceStart:()=>true,returnToHostLobby:()=>true},win:window,doc:document});
window.receive=(channel,value)=>receiver(channel,value);window.audio=window.__privateAudio;
receive('gm_entity',{entities:[{entity_id:'owned-ship',kind:'player_ship',name:'Owned ship'}]});
renderAudioSettingsPanel(document,document.querySelector('main'),audio);
</script></body></html>`;

test('native Station semantic refusal and deliberate output use exact native PCM fixture with live equivalents', {tag:'@core'}, async ({page}) => {
  await page.route('**/client/native-private-station-probe', route=>route.fulfill({contentType:'text/html',body:STATION}));
  const errors=[];page.on('pageerror',error=>errors.push(String(error)));
  await page.goto('/client/native-private-station-probe');
  await expect.poll(()=>page.evaluate(()=>!!document.querySelector('iframe').contentWindow.activateSemanticAction)).toBe(true);
  await page.evaluate(async()=>{await audio.ready;publishAudio();takeAudio();});
  await page.evaluate(()=>document.querySelector('iframe').contentWindow.activateSemanticAction('captain.red-alert',{context:'captain',source:'control'}));
  await expect.poll(()=>page.evaluate(()=>sent[0]?.action)).toBe('set_red_alert');
  await page.evaluate(()=>takeAudio());
  await page.evaluate(()=>document.querySelector('iframe').contentWindow.__updateActionFeedback({correlation:sent[0].correlation,state:'Refused'}));
  await expect(page.frameLocator('iframe').locator('.semantic-action-feedback')).toHaveAttribute('data-state','Refused');
  expect(await page.evaluate(()=>takeAudio())).toBe(refusedFixture);
  await page.evaluate(()=>document.querySelector('iframe').contentWindow.__updateActionFeedback({correlation:sent[0].correlation,state:'Refused'}));
  expect(await page.evaluate(()=>takeAudio())).toBe('');
  await page.locator('[data-audio-test]').click();
  expect(await page.evaluate(()=>takeAudio())).toBe(testFixture);
  await page.evaluate(()=>{publishAudio('failed',2);takeAudio();});
  await expect(page.locator('.audio-output-status')).toContainText('output:Private headset');
  await expect(page.locator('.audio-output-status')).toContainText('disconnected');
  await page.locator('[data-audio-test]').click();expect(await page.evaluate(()=>takeAudio())).toBe('');
  await page.setViewportSize({width:390,height:844});await page.emulateMedia({forcedColors:'active'});
  await page.addStyleTag({content:'body{font-size:200%}iframe{max-width:100%}'});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  expect(errors).toEqual([]);
});

test('actual native GM M8 action uses its own correlated refusal and the same native PCM fixture', {tag:'@core'}, async ({page}) => {
  await page.route('**/client/native-private-gm-probe',route=>route.fulfill({contentType:'text/html',body:GM}));
  const errors=[];page.on('pageerror',error=>errors.push(String(error)));
  await page.goto('/client/native-private-gm-probe');
  await page.evaluate(async()=>{await audio.ready;publishAudio();takeAudio();});
  await page.locator('#gm-presentation-panel > button').nth(1).click();
  expect(await page.evaluate(()=>sent[0])).toMatchObject({action:'presentation',operator_id:'native-gm',cue:'release_view'});
  await page.evaluate(()=>takeAudio());
  await page.evaluate(()=>receive('gm_session',{journal:{entries:[{operator_id:'other',correlation:sent[0].correlation,outcome:'refused'}]}}));
  expect(await page.evaluate(()=>takeAudio())).toBe('');
  await page.evaluate(()=>receive('gm_session',{journal:{entries:[{operator_id:'native-gm',correlation:sent[0].correlation,outcome:'refused'}]}}));
  await page.evaluate(()=>receive('gm_entity',{entities:[{entity_id:'owned-ship',kind:'player_ship',name:'Owned ship'}],presentation_results:[{operator_id:'native-gm',correlation:sent[0].correlation,outcome:'refused'}]}));
  await expect(page.locator('#gm-presentation-panel [role=status]')).toHaveAttribute('data-state','refused');
  expect(await page.evaluate(()=>takeAudio())).toBe(refusedFixture);
  await page.evaluate(()=>receive('gm_session',{journal:{entries:[{operator_id:'native-gm',correlation:sent[0].correlation,outcome:'refused'}]}}));
  expect(await page.evaluate(()=>takeAudio())).toBe('');
  await page.locator('[data-audio-bus=master] button').click();
  await page.evaluate(()=>takeAudio());await page.locator('[data-audio-test]').click();
  expect(await page.evaluate(()=>takeAudio())).toBe('');
  expect(errors).toEqual([]);
});
