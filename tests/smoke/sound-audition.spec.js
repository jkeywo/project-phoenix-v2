import { test, expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import path from 'node:path';
const root=path.resolve(__dirname,'../..');
const boot=readFileSync(path.join(root,'src/native_host/audio/private_boot.js'),'utf8');
const fixture=JSON.parse(readFileSync(path.join(root,'tests/fixtures/native-private-audition.json'),'utf8'));
const customPcm=readFileSync(path.join(root,'tests/fixtures/audio-mono-right.wav'));
const pageHtml=native=>`<!doctype html><html><head><base href='/client/'><link rel='stylesheet' href='/client/gui/audio-settings.css'>
<style>body{margin:8px}#gm-mission-panel{max-width:45rem}fieldset{min-width:0}select{max-width:100%}</style>
${native?`<script>${boot}\nwindow.PhoenixOperatorStorage={isReady:()=>true,getItem:()=>null,setItem:()=>{}};</script>`:''}</head>
<body><section id='gm-mission-panel'></section><script type='module'>
import '/client/gui/strings-boot.js';
import {mountGmWorkspace} from '/client/gui/gm-workspace.js';
import {mountNativeGmWorkspace} from '/client/gui/native-gm-workspace.js';
window.sent=[];window.__phoenixGmPage=true;window.__hostLocalGm=()=>({id:'native-gm',connected:true});
window.workspace=${native?`mountNativeGmWorkspace({win:window,doc:document,bridge:{getOperator:window.__hostLocalGm,submitAction:record=>{sent.push(record);return true;},subscribe:()=>()=>{},setReady:()=>true,forceStart:()=>true,returnToHostLobby:()=>true}})`:'mountGmWorkspace({win:window,doc:document})'};
window.audio=window.__privateAudio;
window.publish=(extra={})=>window.__phoenixPrivateAudioApply(JSON.stringify({generation:1,status:'playing',test:'idle',categories:['alerts','interface'],audition:true,preview:'idle',preview_id:0,surface:'native-gm',outputs:['GM headphones'],...extra}));
window.drain=()=>JSON.parse(window.__phoenixPrivateAudioDrain()||'null');
</script></body></html>`;
test('GM local audition decodes actual packaged MP3, preserves equivalent when muted and stops on close', {tag:'@core'}, async({page})=>{
  const errors=[];page.on('pageerror',e=>errors.push(String(e)));
  await page.route('**/client/sound-audition-probe',route=>route.fulfill({contentType:'text/html',body:pageHtml(false)}));
  await page.goto('/client/sound-audition-probe');
  const panel=page.locator('.sound-audition'),preview=panel.locator('[data-sound-preview]');
  await expect(preview).toBeEnabled();await panel.locator('select').selectOption('weapons');
  await preview.focus();await page.keyboard.press('Enter');
  await expect.poll(()=>page.evaluate(()=>audio.debug().outputPeak)).toBeGreaterThan(0.0001);
  await expect(panel.locator('[data-sound-equivalent]')).toContainText('45');
  await panel.locator('[data-sound-stop]').click();await expect.poll(()=>page.evaluate(()=>audio.debug().outputPeak)).toBeLessThan(0.00001);
  await panel.locator('summary').click();await panel.locator('[data-audio-bus=master] button').click();
  await preview.click();await expect(panel.locator('[data-sound-equivalent]')).toBeVisible();
  expect(await page.evaluate(()=>audio.debug().outputPeak)).toBe(0);expect(await page.evaluate(()=>sent)).toEqual([]);
  await page.setViewportSize({width:390,height:844});await page.emulateMedia({forcedColors:'active'});await page.addStyleTag({content:'body{font-size:200%}'});
  await preview.focus();expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  expect(await preview.evaluate(node=>node===document.activeElement)).toBe(true);
  await page.evaluate(()=>workspace.dispose());expect(errors).toEqual([]);
});
test('actual native GM audition emits the production PCM fixture without a game action and refuses missing output', {tag:'@core'}, async({page})=>{
  const errors=[];page.on('pageerror',e=>errors.push(String(e)));
  await page.route('**/client/native-sound-audition-probe',route=>route.fulfill({contentType:'text/html',body:pageHtml(true)}));
  await page.goto('/client/native-sound-audition-probe');
  const panel=page.locator('.sound-audition'),preview=panel.locator('[data-sound-preview]');await expect(preview).toBeEnabled();
  await page.evaluate(()=>{publish();drain();});await panel.locator('select').selectOption('weapons');
  await preview.click();const record=await page.evaluate(()=>drain());
  expect({...record,generation:0,preview:{...record.preview,at_ms:0}}).toEqual(fixture);
  await page.evaluate(id=>publish({preview:'playing',preview_id:id}),record.preview.id);
  await expect(panel.locator('[data-sound-status]')).toContainText('playing');
  await panel.locator('[data-sound-stop]').click();expect((await page.evaluate(()=>drain())).stop_preview).toBe(true);
  await page.evaluate(()=>{publish({generation:2,status:'failed',outputs:[]});drain();});
  await preview.click();await expect(panel.locator('[data-sound-equivalent]')).toBeVisible();
  const refused=await page.evaluate(()=>drain());expect(refused).not.toHaveProperty('preview');expect(refused).not.toHaveProperty('cue');
  expect(await page.evaluate(()=>sent)).toEqual([]);expect(errors).toEqual([]);
});
test('a new nested packaged sound is decoded locally with its declared equivalent', {tag:'@core'}, async({page})=>{
  const asset={file:'assets/sounds/custom/sonar.wav',category:'alerts',informative:true};
  const cue={id:'sonar',label:'Sonar report',file:asset.file,category:'alerts',audience:'station',volume:0.2,
    equivalent:{meaning:'Report ready',source:'Sensors',urgency:'advisory'}};
  await page.route('**/client/sound-audition-probe',route=>route.fulfill({contentType:'text/html',body:pageHtml(false)}));
  await page.route('**/assets/audio/sound-cues.json',route=>route.fulfill({json:{version:1,assets:[asset],cues:[cue]}}));
  await page.route('**/assets/sounds/custom/sonar.wav',route=>route.fulfill({contentType:'audio/wav',body:customPcm}));
  await page.goto('/client/sound-audition-probe');const preview=page.locator('[data-sound-preview]');await expect(preview).toBeEnabled();
  await preview.click();await expect.poll(()=>page.evaluate(()=>audio.debug().outputPeak)).toBeGreaterThan(0.001);
  await expect(page.locator('[data-sound-equivalent]')).toContainText('Report ready');
  await expect(page.locator('[data-sound-equivalent]')).toHaveAttribute('aria-live','polite');
  expect(await page.evaluate(()=>sent)).toEqual([]);await page.locator('[data-sound-stop]').click();
  await expect.poll(()=>page.evaluate(()=>audio.debug().outputPeak)).toBeLessThan(0.00001);
});
