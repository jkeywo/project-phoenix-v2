// Real classic/deferred-module scheduling around the shipped startup owner.
import { chromium, expect } from '../smoke/node_modules/@playwright/test/index.mjs';
import { readFile } from 'node:fs/promises';
const bootstrap = await readFile(new URL('../../gui/page-startup.js', import.meta.url), 'utf8');
const browser = await chromium.launch({headless:true});
try {
  const page = await browser.newPage(), errors = [];
  page.on('pageerror', error => errors.push(String(error)));
  await page.route('http://startup.test/**', route => {
    if (route.request().url().endsWith('/gui/page-startup.js')) return route.fulfill({contentType:'text/javascript',body:bootstrap});
    return route.fulfill({contentType:'text/html',body:`<!doctype html><script src="/gui/page-startup.js"></script>
      <script>
        PhoenixPageStartup.install(window,{host:true}); window.events = [];
        pageStartup.callChrome('setConnectionStatus','connecting');
        pageStartup.callAudio('startGameAudio'); __hostChannel('hud',1); __hostChannel('audio_config',2);
      </script>
      <script type="module">
        await new Promise(resolve => {window.finishAudio = resolve;});
        __hostAudioReady({startGameAudio:()=>events.push('audio-start'),setPageActive:()=>{},dispose:()=>events.push('audio-dispose')});
      </script>
      <script type="module">
        __hostChannelReady((name,payload)=>events.push([name,payload]));
        __pageChromeReady({setConnectionStatus:state=>events.push(state),releaseWakeLock:()=>{}});
        window.collaboratorsMounted = true;
      </script>`});
  });
  await page.goto('http://startup.test/',{waitUntil:'domcontentloaded'});
  await expect.poll(() => page.evaluate(() => window.collaboratorsMounted)).toBe(true);
  expect(await page.evaluate(() => events)).toEqual(['connecting']);
  await page.evaluate(() => finishAudio());
  await expect.poll(() => page.evaluate(() => events)).toEqual(['connecting','audio-start',['hud',1],['audio_config',2]]);
  await page.evaluate(() => {pageStartup.dispose(); __hostChannel('hud',3);});
  expect(await page.evaluate(() => events)).toEqual(['connecting','audio-start',['hud',1],['audio_config',2],'audio-dispose']);
  expect(errors).toEqual([]);
  console.log('PASS classic bootstrap, delayed module readiness, ordered Host Channel/audio and teardown');
} finally {await browser.close();}
