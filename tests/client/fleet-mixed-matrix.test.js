import {describe,it,expect} from 'vitest';
import vm from 'node:vm';
import {mixedOptions,mixedNativeSeconds,mixedBrowserRuntime,mixedOutcome,observeMixedDigests,verifyMixedImpairment} from '../../scripts/fleet-mixed-matrix.mjs';
import {observeBrowser} from '../../scripts/fleet-browser-matrix.mjs';
function fixture(route='direct'){
 const ids=['ship-1','ship-2','gm-1'];
 const rtc=()=>({connectionState:'connected',selected:[{state:'succeeded',bytesReceived:10,localType:'host',remoteType:'host'}]});
 const browser=ids.concat(['ship-1','ship-2'].flatMap(id=>['captain','helm','engineering'].map(s=>id+'/'+s))).map(label=>({label,errors:[],state:{phase:'InProgress',fleet:{role:label.startsWith('gm')?'gm':'ship'},mesh:{slot:ids.indexOf(label)+1,in_fleet:true,peers:[1,2,3,4,5],peers_heard:[1,2,3,4,5],samples:2,agreed:true},mixedGmActions:['pause','resume'],mixedDigests:[300,600].map(tick=>({at:'2026-09-27T12:00:01Z',value:{tick,digest:'abc'}})),outcomes:[{correlation:'mixed-example',outcome:'Applied'}],counts:{'relay-peer':label==='ship-1'?(route==='direct'?3:8):0},relayReady:route==='direct'||label==='ship-1'?0:1,relayFrames:20,signalOffersSent:route==='ws-relay'?0:1,rtc:route==='direct'?Array.from({length:label==='ship-1'?5:label==='ship-2'?4:1},rtc):[]}}));
 const events=Object.fromEntries(['ship-3','ship-4','gm-2'].map(id=>[id,[{kind:'simulation-roster',value:{generation:1,local:['ship-3','ship-4','gm-2'].indexOf(id)+4,shipHosts:[1,2,4,5],gmHosts:[3,6]}},{kind:'state',value:{roster_result:{generation:1,accepted:true}}},{kind:'fleet_join_status',value:{status:'admitted'}},{kind:'onRoster',value:{participants:[1,2,3,4,5,6],slots:[1,2,3,4]}},{kind:'onDiag',value:{event:'transport',transport:'ws-relay'}},...[300,600].map(tick=>({kind:'digest',at:'2026-09-27T12:00:01Z',value:{tick,digest:'abc'}}))]]));
 for(const id of ['ship-3','ship-4'])for(const station of ['captain','helm','engineering'])events[id].push(...['assigned','ready','started'].map(kind=>({kind:'station-'+kind,value:{station,[kind]:true}})),...[1,2].flatMap(n=>[{kind:'station-command',value:{station,correlation:station+n}},{kind:'station-feedback',value:{station,correlation:station+n,outcome:'Applied'}}]));
 events['gm-2'].push({kind:'gm-metadata',value:{local_operator_id:'operator'}},...[1,2].flatMap(n=>[{kind:'gm-action-requested',value:{accepted:true,correlation:'gm'+n}},{kind:'gm-activity',value:{entries:[{detail:{type:'gm_action',data:{operator:{id:'operator'},outcome:'applied',correlation:'gm'+n}}}]}}]));
 return {browser,events,options:{route,digestAfter:'2026-09-27T12:00:00Z',commandWaves:1}};
}
describe('mixed runtime evidence gate',()=>{
 it.each(['direct','ws-relay','automatic-fallback'])('accepts complete %s observations',route=>{const f=fixture(route);expect(mixedOutcome(f.browser,f.events,f.options).passed).toBe(true);});
 it('requires native receipts and browser receipts separately',()=>{const f=fixture();f.events['ship-4']=f.events['ship-4'].filter(e=>e.kind!=='station-feedback');f.browser.at(-1).state.outcomes=[];const r=mixedOutcome(f.browser,f.events,f.options);expect(r.passed).toBe(false);expect(r.reasons.some(s=>s.includes('ship-4/'))).toBe(true);expect(r.reasons.some(s=>s.includes('command receipts'))).toBe(true);});
 it('requires two matching digests after the workload boundary from all runtimes',()=>{const f=fixture();f.events['gm-2'].find(e=>e.kind==='digest').value.digest='different';expect(mixedOutcome(f.browser,f.events,f.options).passed).toBe(false);f.events['gm-2'].find(e=>e.kind==='digest').value.digest='abc';f.options.digestAfter='2026-09-27T12:00:02Z';expect(mixedOutcome(f.browser,f.events,f.options).passed).toBe(false);});
 it('rejects browser relay in the direct-capable subset',()=>{const f=fixture();f.browser[1].state.relayReady=1;expect(mixedOutcome(f.browser,f.events,f.options).passed).toBe(false);});
 it('requires observed native relay even when browsers are direct',()=>{const f=fixture();f.events['ship-3']=f.events['ship-3'].filter(e=>e.kind!=='onDiag');expect(mixedOutcome(f.browser,f.events,f.options).passed).toBe(false);});
 it('requires actual fallback offers',()=>{const f=fixture('automatic-fallback');f.browser[1].state.signalOffersSent=0;expect(mixedOutcome(f.browser,f.events,f.options).passed).toBe(false);});
 it('observes returned digest frames without draining or changing raw data',()=>{const context=vm.createContext({window:{}});vm.runInContext('('+observeMixedDigests.toString()+')()',context);let calls=0;const raw=JSON.stringify([{t:'digest',d:{tick:300,digest:'abc'}},{t:'tick',d:{tick:301}}]);context.window.wasm_take_mesh_frames=()=>{calls++;return raw;};expect(context.window.wasm_take_mesh_frames()).toBe(raw);expect(calls).toBe(1);expect(context.window.__mixedDigests).toHaveLength(1);});
 it('requires distinct adopted slots and exact roles',()=>{const f=fixture();f.events['gm-2'][0].value.local=4;expect(mixedOutcome(f.browser,f.events,f.options).reasons).toContain('Six distinct adopted simulation slots required');expect(mixedOutcome(f.browser,f.events,f.options).reasons).toContain('Exact four ship and two GM roles required');});
 it('refuses failed native telemetry even with sufficient captured workload',()=>{const f=fixture();f.events['gm-2'].push({kind:'observer-error',value:{status:431}});expect(mixedOutcome(f.browser,f.events,f.options).passed).toBe(false);});
 it('bounds run options and requires native inputs',()=>{expect(()=>mixedOptions(['--out','unused'])).toThrow('binary');expect(()=>mixedOptions(['--out','unused','--binary','bin','--bundle','dist','--deadline','901'])).toThrow('deadline');});
 it('refuses claimed impairment without actual native relay writes',()=>{expect(()=>verifyMixedImpairment({route:'direct',impairment:{profile:{delay_ms:0,loss_percent:0,seed:1530},counters:{relay_reliable_written:0}}},{delayMs:0,lossPercent:0,seed:1530})).toThrow('native relay traffic');});
});

describe('mixed browser render mode', () => {
 const required = ['--out', 'unused', '--binary', 'bin', '--bundle', 'dist'];
 it.each([0, 2, 6])('accepts the standalone render flag at argument %i without consuming another option', index => {
  const argv = [...required]; argv.splice(index, 0, '--render');
  const options = mixedOptions([...argv, '--seconds', '3']);
  expect(options.render).toBe(true); expect(options.seconds).toBe(3);
 });
 it.each([false, true])('sets the webdriver flag used for browser boot selection when render=%s', render => {
  const options = mixedOptions([...required, ...(render ? ['--render'] : []), '--delay-ms', '20', '--loss-percent', '10']);
  const runtime = mixedBrowserRuntime(options, 'direct');
  const context = vm.createContext({ navigator: {webdriver:true}, document: {addEventListener(){}}, window: {addEventListener(){}, WebSocket:class {}, RTCPeerConnection:class {}}, observer:runtime.observer });
  vm.runInContext('('+observeBrowser.toString()+')(observer,()=>{})', context);
  expect(context.navigator.webdriver).toBe(!render);
  expect(runtime.launch.args.includes('--use-angle=swiftshader')).toBe(render);
  expect(runtime.launch.args.includes('--enable-unsafe-swiftshader')).toBe(render);
  expect(runtime.observer.directProfile).toEqual({delayMs:20,lossPercent:10,seed:1530});
  expect(mixedBrowserRuntime(options, 'ws-relay').observer.directProfile).toBeNull();
  expect(runtime.limitation).toContain(render ? 'Software rendering requested' : 'rendering disabled');
 });
 it('keeps malformed value options invalid beside the render flag', () => {
  expect(()=>mixedOptions([...required, '--seconds', '--render'])).toThrow('Incomplete option');
  expect(()=>mixedOptions([...required, '--render', 'false'])).toThrow('Incomplete option');
 });
});

it('extends native lifetime only for explicitly longer bounded mixed runs',()=>{
  const required=['--out','unused','--binary','bin','--bundle','dist'];
  expect(mixedOptions(required).deadline).toBe(290);
  expect(mixedNativeSeconds(mixedOptions(required).deadline)).toBe(300);
  expect(mixedNativeSeconds(295)).toBe(300);
  expect(mixedNativeSeconds(mixedOptions([...required,'--deadline','900']).deadline)).toBe(960);
  expect(()=>mixedNativeSeconds(901)).toThrow();
  expect(()=>mixedOptions([...required,'--deadline','900.5'])).toThrow();
});
