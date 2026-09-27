#!/usr/bin/env node
// Real mixed fleet: two browser ships, two native ships, one GM in each runtime.
import fs from 'node:fs/promises';
import path from 'node:path';
import { createServer } from 'node:http';
import { spawn, execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { observeBrowser, bundleHashes, ROUTES } from './fleet-browser-matrix.mjs';
import { impairDataChannel } from './fleet-channel-impairment.mjs';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const nativeIds = ['ship-3', 'ship-4', 'gm-2'];
const browserIds = ['ship-1', 'ship-2', 'gm-1'];
const stationNames = ['captain', 'helm', 'engineering'];
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const sha = data => createHash('sha256').update(data).digest('hex');
export function mixedOptions(argv) {
  const o = {dist:path.join(root,'dist'), dependencies:path.join(root,'tests/smoke'), source:root, seconds:10, timeout:120, deadline:290, port:18450, rendezvousPort:18451, delayMs:0, lossPercent:0, seed:1530, routes:[...ROUTES]};
  const paths = ['out','dist','dependencies','binary','bundle','source','native-adapter','service-script','build-receipt'];
  const numbers = {seconds:'seconds',timeout:'timeout',deadline:'deadline',port:'port','rendezvous-port':'rendezvousPort','delay-ms':'delayMs','loss-percent':'lossPercent',seed:'seed'};
  for (let i=0;i<argv.length;i+=2) {
    const key=argv[i]?.slice(2),value=argv[i+1];
    if (!argv[i]?.startsWith('--') || !value || value.startsWith('--')) throw new Error('Incomplete option');
    if(paths.includes(key))o[key]=path.resolve(value);
    else if(key==='routes')o.routes=value.split(',');
    else if(numbers[key])o[numbers[key]]=Number(value);
    else throw new Error('Unknown option '+key);
  }
  for(const key of ['out','binary','bundle'])if(!o[key])throw new Error('--'+key+' is required');
  for(const [key,min,max] of [['seconds',1,60],['timeout',1,180],['deadline',30,295],['port',1024,65535],['rendezvousPort',1024,65535],['delayMs',0,2000],['lossPercent',0,99],['seed',0,4294967295]])if(!Number.isInteger(o[key])||o[key]<min||o[key]>max)throw new Error('Invalid '+key);
  if(o.port===o.rendezvousPort)throw new Error('Ports must differ');
  if(!o.routes.length||o.routes.some(r=>!ROUTES.includes(r))||new Set(o.routes).size!==o.routes.length)throw new Error('Invalid routes');
  return o;
}
function has(rows,kind,predicate=()=>true){return rows?.some(row=>row.kind===kind&&predicate(row.value))||false;}
export function mixedOutcome(browserRows,events,{route,digestAfter,commandWaves=1}) {
  const reasons=[], browser=Object.fromEntries(browserRows.map(row=>[row.label,row]));
  const need=(condition,reason)=>{if(!condition)reasons.push(reason);};
  const native={}, localSlots=[], roles=[];
  for(const id of nativeIds) {
    const rows=events[id]||[],roster=rows.filter(r=>r.kind==='onRoster').at(-1)?.value;
    const errors=rows.filter(r=>['fleet_fault','page-error','page-rejection','configure-error','station-error','gm-control-error','observer-error','observer-overflow'].includes(r.kind));
    const adopted=rows.filter(r=>r.kind==='simulation-roster'&&has(rows,'state',v=>v.roster_result?.generation===r.value.generation&&v.roster_result.accepted)).at(-1)?.value;
    const role=adopted?(adopted.shipHosts?.includes(adopted.local)&&!adopted.gmHosts?.includes(adopted.local)?'ship':adopted.gmHosts?.includes(adopted.local)&&!adopted.shipHosts?.includes(adopted.local)?'gm':null):null;
    localSlots.push(adopted?.local);roles.push(role);
    need(role===(id.startsWith('gm')?'gm':'ship'),id+': adopted role missing or incorrect');
    native[id]={localSlot:adopted?.local,role,admitted:has(rows,'fleet_join_status',v=>v.status==='admitted'),roster,errors,relay:has(rows,'onDiag',v=>v.event==='transport'&&v.transport==='ws-relay')};
    need(native[id].admitted&&roster?.participants?.length===6&&roster?.slots?.length===4,id+': admission/roster incomplete');
    need(native[id].relay,id+': actual relay unobserved');need(!errors.length,id+': runtime errors');
  }
  const stations=['ship-3','ship-4'].flatMap(id=>stationNames.map(station=>{
    const rows=events[id]||[],requests=rows.filter(r=>r.kind==='station-command'&&r.value.station===station);
    const applied=new Set(rows.filter(r=>r.kind==='station-feedback'&&r.value.station===station&&r.value.outcome==='Applied'&&requests.some(q=>q.value.correlation===r.value.correlation)).map(r=>r.value.correlation));
    const refused=rows.filter(r=>r.kind==='station-feedback'&&r.value.station===station&&r.value.outcome==='Refused');
    const state={id,station,assigned:has(rows,'station-assigned',v=>v.station===station&&v.assigned),ready:has(rows,'station-ready',v=>v.station===station&&v.ready),started:has(rows,'station-started',v=>v.station===station),commands:requests.length,applied:applied.size,refused:refused.length};
    need(state.assigned&&state.ready&&state.started&&state.applied>=2&&!state.refused,id+'/'+station+': active Station evidence incomplete');return state;
  }));
  const gmRows=events['gm-2']||[],operator=gmRows.filter(r=>r.kind==='gm-metadata').at(-1)?.value.local_operator_id;
  const requests=gmRows.filter(r=>r.kind==='gm-action-requested'&&r.value.accepted);
  const applied=new Set(gmRows.filter(r=>r.kind==='gm-activity').flatMap(r=>r.value.entries||[]).filter(e=>e.detail?.type==='gm_action'&&e.detail.data.operator?.id===operator&&e.detail.data.outcome==='applied'&&requests.some(q=>q.value.correlation===e.detail.data.correlation)).map(e=>e.detail.data.correlation));
  need(applied.size>=2,'gm-2: authoritative GM action receipts incomplete');
  for(const id of browserIds){const row=browser[id],s=row?.state;localSlots.push(s?.mesh?.slot);roles.push(s?.fleet?.role);need(s?.mesh?.in_fleet&&s.mesh.peers.length===5&&s.mesh.samples>0&&s.mesh.peers_heard.length===5&&s.mesh.agreed&&!s.mesh.disagreement&&s.phase==='InProgress',id+': simulation evidence incomplete');need(s?.fleet?.role===(id.startsWith('gm')?'gm':'ship'),id+': wrong role');}
  need(localSlots.length===6&&localSlots.every(s=>Number.isSafeInteger(s)&&s>0)&&new Set(localSlots).size===6,'Six distinct adopted simulation slots required');
  need(roles.filter(r=>r==='ship').length===4&&roles.filter(r=>r==='gm').length===2,'Exact four ship and two GM roles required');
  need(browserRows.length===9,'Exactly three browser simulations and six browser Station documents required');
  const clients=browserRows.filter(r=>r.label.includes('/'));
  need(clients.length===6,'Expected six browser Station documents');
  for(const row of clients){const receipts=row.state?.outcomes||[];need(row.state?.phase==='InProgress'&&receipts.filter(r=>r.outcome==='Applied'&&r.correlation?.startsWith('mixed-')).length>=Math.min(commandWaves,64)&&!receipts.some(r=>r.outcome==='Refused'&&r.correlation?.startsWith('mixed-')),row.label+': command receipts incomplete');}
  for(const row of browserRows){
    const s=row.state,active=s?.rtc?.filter(p=>p.connectionState==='connected')||[];
    need(!row.errors?.length&&!s?.digestOverflow,row.label+': browser/observer error');
    if(route==='direct') {
      const count=row.label==='ship-1'?5:row.label==='ship-2'?4:1;
      need(active.length===count&&active.every(p=>p.selected.some(pair=>pair.state==='succeeded'&&pair.bytesReceived>0&&['host','srflx','prflx'].includes(pair.localType)&&['host','srflx','prflx'].includes(pair.remoteType))),row.label+': direct links unobserved');
      need(row.label==='ship-1'?s?.counts?.['relay-peer']===3:!s?.relayReady&&!s?.counts?.['relay-peer'],row.label+': unexpected browser relay');
    } else {
      need(!active.length&&s?.relayFrames>0&&(row.label==='ship-1'?s?.counts?.['relay-peer']===8:s?.relayReady>0),row.label+': relay links unobserved');
      if(route==='ws-relay')need(!s?.signalOffersSent,row.label+': forced relay attempted RTC');
      if(route==='automatic-fallback'&&row.label!=='ship-1')need(s?.signalOffersSent>0,row.label+': fallback never attempted RTC');
    }
  }
  need(browser['gm-1']?.state?.mixedGmActions?.length===2,'gm-1: pause/resume observations incomplete');
  const ticks=new Map();let conflict=false;
  const record=(id,d)=>{if(!Number.isSafeInteger(d?.tick)||d.tick<=0||typeof d.digest!=='string'||!d.digest){reasons.push(id+': malformed outgoing digest');return;}if(!ticks.has(d.tick))ticks.set(d.tick,new Map());const m=ticks.get(d.tick);if(m.has(id)&&m.get(id)!==d.digest)conflict=true;m.set(id,d.digest);};
  for(const id of browserIds)for(const row of browser[id]?.state?.mixedDigests||[])if(row.at>digestAfter)record(id,row.value);
  for(const id of nativeIds)for(const row of events[id]||[])if(row.kind==='digest'&&row.at>digestAfter)record(id,row.value);
  const commonDigests=[...ticks].filter(([,v])=>v.size===6).map(([tick,v])=>({tick,byPeer:Object.fromEntries(v),agreed:new Set(v.values()).size===1}));
  need(commonDigests.length>=2&&!conflict&&commonDigests.every(d=>d.agreed),'Two matching post-workload six-peer digests not observed');
  return {passed:!reasons.length,reasons,native,stations,nativeGm:{operator,requests:requests.length,applied:applied.size},commonDigests};
}
// Observe the actual outbound digest return value without draining additional frames.
export function observeMixedDigests(){
  window.__mixedDigests=[];window.__mixedDigestOverflow=false;
  Object.defineProperty(window,'wasm_take_mesh_frames',{configurable:true,set(fn){Object.defineProperty(window,'wasm_take_mesh_frames',{configurable:true,writable:true,value:function(...args){const raw=fn.apply(this,args);try{for(const frame of JSON.parse(raw))if(frame.t==='digest'){if(window.__mixedDigests.length>=64)window.__mixedDigestOverflow=true;else window.__mixedDigests.push({at:new Date().toISOString(),value:frame.d});}}catch{}return raw;}});}});
}
async function within(promise,ms,label){let timer;try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error(label+' timed out')),ms);})]);}finally{clearTimeout(timer);}}
async function stopProcess(child){if(!child||child.exitCode!==null||child.signalCode!==null)return;const closed=new Promise(r=>child.once('close',r));child.kill();try{await within(closed,3000,'process shutdown');}catch{if(process.platform==='win32')execFileSync('taskkill',['/PID',String(child.pid),'/T','/F'],{windowsHide:true,timeout:5000,stdio:'ignore'});else child.kill('SIGKILL');await within(closed,3000,'forced process shutdown');}}
async function serve(directory,port){const mime={'.html':'text/html','.js':'text/javascript','.wasm':'application/wasm','.json':'application/json','.css':'text/css','.toml':'text/plain'};const server=createServer(async(req,res)=>{try{let p=path.resolve(directory,'.'+decodeURIComponent(new URL(req.url,'http://localhost').pathname));if(p!==directory&&!p.startsWith(directory+path.sep)){res.writeHead(403).end();return;}if((await fs.stat(p)).isDirectory())p=path.join(p,'index.html');res.setHeader('Content-Type',mime[path.extname(p)]||'application/octet-stream');res.end(await fs.readFile(p));}catch{res.writeHead(404).end();}});await new Promise((r,j)=>{server.once('error',j);server.listen(port,'127.0.0.1',r);});return server;}
export function verifyMixedImpairment(result,options){
  const p=result.impairment?.profile,c=result.impairment?.counters;
  if(!p||p.delay_ms!==options.delayMs||p.loss_percent!==options.lossPercent||p.seed!==options.seed)throw new Error('Actual relay profile differs');
  if(result.route==='automatic-fallback'&&(!p.block_rtc_offers||c.signal_offers_dropped<1))throw new Error('No suppressed offers observed');
  if(c.queue_overflow_closes)throw new Error('Relay queue overflow');
  if(options.lossPercent&&(result.route!=='direct'||c.relay_snapshot_seen>0)&&!c.relay_snapshot_dropped)throw new Error('No relay snapshot drops');
  if(!c.relay_reliable_written)throw new Error('No actual native relay traffic');
  if(options.delayMs&&(!c.observed_write_delay_ms?.reliable.count||c.observed_write_delay_ms.reliable.min<options.delayMs-1))throw new Error('Relay write delay not observed');
  if(result.route==='direct'&&(options.delayMs||options.lossPercent)){
    const counters=result.browser.map(r=>r.state?.directImpairment||{});
    if(counters.some(c=>c.failures||c.overflows))throw new Error('Direct queue failure');
    const reliable=counters.map(c=>c.reliable).filter(Boolean),snapshots=counters.map(c=>c.snapshot).filter(Boolean);
    if(!reliable.some(c=>c.written>0)||!snapshots.some(c=>c.written>0))throw new Error('Direct game writes absent');
    if(options.lossPercent&&!snapshots.some(c=>c.dropped>0))throw new Error('No direct snapshot drops');
    if(options.delayMs&&[...reliable,...snapshots].some(c=>c.written&&(!c.delay.count||c.delay.min<options.delayMs-1)))throw new Error('Direct write delay not observed');
  }
}
async function runCase(browser,o,route,runNativeProbe){
  const directory=path.join(o.out,route);await fs.mkdir(directory);
  const result={route,startedUtc:new Date().toISOString(),status:'running',browser:[],nativeEvents:{},steps:[]};
  const contexts=[],pages=[],nativeRuns=[],nativeCommands=[];let stop=false,nativeFailure;
  const base=`http://127.0.0.1:${o.port}`,rendezvous=`http://127.0.0.1:${o.rendezvousPort}`;
  const script=o['service-script']||path.join(root,'scripts/rendezvous-dev-server.mjs');
  const args=[script,'--port',String(o.rendezvousPort),'--delay-ms',String(o.delayMs),'--loss-percent',String(o.lossPercent),'--seed',String(o.seed),...(route==='automatic-fallback'?['--block-rtc-offers']:[])];
  result.service={args,sha256:sha(await fs.readFile(script))};
  const service=spawn(process.execPath,args,{cwd:root,windowsHide:true,stdio:['ignore','pipe','pipe']});let serviceLog='';const log=b=>{serviceLog=(serviceLog+b).slice(-100000);};service.stdout.on('data',log);service.stderr.on('data',log);service.on('error',log);
  const deadline=Date.now()+o.deadline*1000;
  const step=name=>{result.steps.push({name,utc:new Date().toISOString()});process.stderr.write(route+': '+name+'\n');};
  const wait=async(predicate,label)=>{while(!(await predicate())){if(nativeFailure)throw new Error(nativeFailure);if(Date.now()>deadline)throw new Error('Mixed deadline: '+label);await sleep(150);}};
  async function page(label){const context=await browser.newContext();contexts.push(context);await context.addInitScript({content:`(${observeBrowser.toString()})(${JSON.stringify({render:false,directProfile:route==='direct'&&(o.delayMs||o.lossPercent)?{delayMs:o.delayMs,lossPercent:o.lossPercent,seed:o.seed}:null})},${impairDataChannel.toString()});(${observeMixedDigests.toString()})();`});const page=await context.newPage();page.setDefaultTimeout(o.timeout*1000);const row={label,page,errors:[],logs:[]};pages.push(row);page.on('pageerror',e=>{row.errors.push(String(e).slice(0,2000));if(row.errors.length>100)row.errors.shift();});page.on('console',m=>{if(['warning','error'].includes(m.type())){row.logs.push(m.text().slice(0,2000));if(row.logs.length>100)row.logs.shift();}});return page;}
  const evaluate=(page,fn,arg)=>within(page.evaluate(fn,arg),o.timeout*1000,'browser evaluation');
  const query=new URLSearchParams({rendezvous,...(route==='automatic-fallback'?{}:{transport:route})});
  const capture=async()=>Promise.all(pages.map(async row=>{const snapshot={label:row.label,errors:[...row.errors],logs:[...row.logs]};try{snapshot.state=await evaluate(row.page,async()=>({...await window.__matrixRead(),mixedDigests:window.__mixedDigests,digestOverflow:window.__mixedDigestOverflow,mixedGmActions:window.__mixedGmActions}));}catch(e){snapshot.readError=String(e);}return snapshot;}));
  try{
    await wait(async()=>{if(service.exitCode!==null)throw new Error('Rendezvous exited');try{return(await fetch(rendezvous+'/v1/health',{signal:AbortSignal.timeout(2000)})).ok;}catch{return false;}},'service');step('service ready');
    const owner=await page('ship-1');await owner.goto(`${base}/?${query}&scenario=assets/worlds/probe_fleet_six_peer.toml&ship=assets/entities/alliance_cruiser.toml`);await owner.waitForFunction(()=>window.__matrixPhoenixReady&&window.__matrixEvidence.counts.hosted>0);await evaluate(owner,()=>window.__hostFleetOpen());await owner.waitForFunction(()=>window.__hostFleetState?.().suffix);const code=await evaluate(owner,()=>window.__hostFleetState().suffix);step('browser owner ready');
    const launchNative=id=>{result.nativeEvents[id]=[];nativeRuns.push(runNativeProbe({binary:o.binary,bundle:o.bundle,source:o.source,out:path.join(directory,id),rendezvous,origin:base,seconds:300,role:id.startsWith('gm')?'gm':'ship','fleet-code':code,workload:true,deferGmReady:id==='gm-2',nextCommand:()=>id==='gm-2'?nativeCommands.shift():null,shouldStop:()=>stop,onEvent:event=>{if(!stop){result.nativeEvents[id].push(event);if(['fleet_fault','page-error','page-rejection','configure-error','observer-error','observer-overflow'].includes(event.kind))nativeFailure=id+': '+event.kind+' '+JSON.stringify(event.value);}}}).then(r=>{if(!stop)nativeFailure=id+': native process ended before mixed verdict';return r;}).catch(e=>{nativeFailure=id+': '+e;return {failure:String(e)};}));};
    launchNative('ship-3');launchNative('ship-4');launchNative('gm-2');
    const ships=[owner];let gm;
    await Promise.all(['ship-2','gm-1'].map(async id=>{const p=await page(id);if(id==='ship-2')ships.push(p);else gm=p;await p.goto(`${base}/?${query}&scenario=assets/worlds/probe_fleet_six_peer.toml&${id==='gm-1'?'gm=1':'ship=assets/entities/alliance_cruiser.toml'}`);await p.waitForFunction(()=>window.__matrixPhoenixReady&&window.__matrixEvidence.counts.hosted>0);await evaluate(p,code=>window.__hostFleetJoin(code),code);await p.waitForFunction(()=>window.__hostFleetState?.().open);if(id==='gm-1')await p.waitForFunction(()=>window.__hostGmStartState?.().admitted&&window.__hostGmStartState?.().localValidation);}));
    await wait(()=>nativeIds.every(id=>has(result.nativeEvents[id],'fleet_join_status',v=>v.status==='admitted')),'native admission');step('all six peers admitted');
    const clients=[];
    await Promise.all(ships.flatMap((ship,i)=>stationNames.map(async station=>{const code=(await ship.locator('#join-code').textContent()).trim();const p=await page(`ship-${i+1}/${station}`);await p.goto(`${base}/client/?${query}#${code}`);await p.waitForFunction(()=>window.phoenixLink?.connected&&window.lobbyState?.players?.length>0);await evaluate(p,s=>window.phoenixLink.send('SelectStation',{station:s},'reliable'),station);await p.waitForFunction(s=>window.lobbyState.players.some(p=>p.station===s&&p.connected),station);await evaluate(p,()=>window.phoenixLink.send('SetReady',{ready:true},'reliable'));clients.push({page:p,station,ship:i+1});})));
    nativeCommands.push({kind:'ready'});
    await evaluate(gm,()=>document.getElementById('gm-ready-btn').click());await Promise.all([...ships,gm].map(p=>p.waitForFunction(()=>window.__saveSlotsPhase==='InProgress'&&window.__hostMeshStatus?.().in_fleet)));step('mixed automatic launch');
    let wave=0;const until=Date.now()+o.seconds*1000;
    while(Date.now()<until){for(const c of clients)await evaluate(c.page,({station,ship,wave})=>{const correlation=`mixed-${ship}-${station}-${wave}`;const action=station==='captain'?{action:'set_red_alert',active:wave%2===0,correlation}:station==='helm'?{action:'set_boost',active:wave%2===0,correlation}:{action:'set_power',target:'weapons',level:wave%2?2:3,correlation};const send=(type,data)=>window.phoenixLink.send(type,data,'reliable');window.dispatchConsoleAction(action,send);if(station==='helm')window.dispatchConsoleAction({action:'set_helm_thrust',value:.2},send);},{station:c.station,ship:c.ship,wave});wave++;await sleep(500);}
    result.commandWaves=wave;
    await evaluate(gm,()=>document.getElementById('gm-session-pause').click());await gm.waitForFunction(()=>window.__hostGmSessionState?.().paused);await wait(()=>has(result.nativeEvents['gm-2'],'gm-session',v=>v.paused),'native GM sees pause');
    const resumeAt=new Date().toISOString();await evaluate(gm,()=>document.getElementById('gm-session-resume').click());await gm.waitForFunction(()=>!window.__hostGmSessionState?.().paused);await wait(()=>result.nativeEvents['gm-2'].some(r=>r.at>resumeAt&&r.kind==='gm-session'&&!r.value.paused),'native GM sees resume');await evaluate(gm,()=>{window.__mixedGmActions=['pause observed in both runtimes','resume observed in both runtimes'];});
    result.digestAfter=new Date().toISOString();step('active commands and browser GM actions complete');
    await wait(async()=>{result.browser=await capture();result.outcome=mixedOutcome(result.browser,result.nativeEvents,{route,digestAfter:result.digestAfter,commandWaves:wave});if(result.outcome.commonDigests.some(d=>!d.agreed))throw new Error('Observed mixed runtime digest disagreement');if(Object.values(result.outcome.native).some(p=>p.errors.length)||result.browser.some(p=>p.errors.length))throw new Error('Observed mixed runtime error');if(!result.outcome.passed)await sleep(850);return result.outcome.passed;},'post-workload six-peer digest agreement');
    result.impairment=await(await fetch(rendezvous+'/__impairment',{signal:AbortSignal.timeout(5000)})).json();verifyMixedImpairment(result,o);result.status='passed';step('two matching post-workload digests observed');
  }catch(e){result.status='failed';result.error=String(e.stack||e);result.browser=await capture();result.outcome=mixedOutcome(result.browser,result.nativeEvents,{route,digestAfter:result.digestAfter||result.startedUtc,commandWaves:result.commandWaves||1});}
  finally{
    stop=true;
    if(!result.impairment)try{result.impairment=await(await fetch(rendezvous+'/__impairment',{signal:AbortSignal.timeout(5000)})).json();}catch{result.impairment=null;}
    if(result.status!=='passed')for(const row of pages)try{await row.page.screenshot({path:path.join(directory,row.label.replaceAll('/','-')+'.png'),timeout:3000});}catch{}
    await Promise.allSettled(contexts.map(c=>within(c.close(),5000,'context shutdown')));
    result.nativeRuns=await within(Promise.all(nativeRuns),20000,'native teardown').catch(e=>{result.status='failed';result.cleanupError=String(e);return[];});
    if(result.nativeRuns.some(r=>r.failure||!r.outcome?.bootstrapObserved||r.binaryUnchanged!==true)){result.status='failed';result.cleanupError||='Native bootstrap or teardown failed';}
    try{await stopProcess(service);}catch(e){result.status='failed';result.cleanupError=String(e);}
    await fs.writeFile(path.join(directory,'service.log'),serviceLog);result.finishedUtc=new Date().toISOString();await fs.writeFile(path.join(directory,'result.json'),JSON.stringify(result,null,2));
  }return result;
}
export async function main(argv=process.argv.slice(2)){
  const o=mixedOptions(argv);await fs.mkdir(path.dirname(o.out),{recursive:true});await fs.mkdir(o.out);
  const adapter=o['native-adapter']?pathToFileURL(o['native-adapter']).href:new URL('./fleet-native-probe.mjs',import.meta.url).href;
  const {runNativeProbe}=await import(adapter);const require=createRequire(path.join(o.dependencies,'package.json'));const {chromium}=require('@playwright/test');
  const manifest={kind:'real mixed browser/native fleet matrix',status:'running',options:o,sourceRevision:execFileSync('git',['rev-parse','HEAD'],{cwd:root,encoding:'utf8'}).trim(),sourcePatch:execFileSync('git',['diff','HEAD'],{cwd:root,encoding:'utf8'}),runnerSha256:sha(await fs.readFile(fileURLToPath(import.meta.url))),browserHarnessSha256:sha(await fs.readFile(path.join(root,'scripts/fleet-browser-matrix.mjs'))),channelImpairmentSha256:sha(await fs.readFile(path.join(root,'scripts/fleet-channel-impairment.mjs'))),nativeAdapterSha256:sha(await fs.readFile(fileURLToPath(adapter))),binarySha256:sha(await fs.readFile(o.binary)),browserBundleHashes:await bundleHashes(o.dist),nativeBundleHashes:await bundleHashes(o.bundle),limits:['Single-machine loopback','Browser WASM rendering disabled; native Bevy and Ultralight remain real','Six embedded native Station documents plus six browser Station documents','Application-frame impairment; not IP packet loss','Bounded acceptance subset; no mobile/endurance/performance/recovery acceptance'],results:[]};
  if(o['build-receipt']){const raw=await fs.readFile(o['build-receipt']);const receipt=JSON.parse(raw);manifest.nativeBuildReceipt={path:o['build-receipt'],sha256:sha(raw),receipt,binaryMatchesRecordedReceipt:receipt.binarySha256===manifest.binarySha256};if(!manifest.nativeBuildReceipt.binaryMatchesRecordedReceipt)throw new Error('Native binary does not match build receipt');}
  const save=()=>fs.writeFile(path.join(o.out,'manifest.json'),JSON.stringify(manifest,null,2));await save();let server,browser,browserServer;
  try{server=await serve(o.dist,o.port);browserServer=await chromium.launchServer({headless:true,args:['--autoplay-policy=no-user-gesture-required','--disable-background-timer-throttling','--disable-renderer-backgrounding']});browser=await chromium.connect(browserServer.wsEndpoint(),{timeout:30000});manifest.browser=browser.version();await save();for(const route of o.routes){const r=await runCase(browser,o,route,runNativeProbe);manifest.results.push({route,status:r.status,error:r.error});await save();if(r.status!=='passed')break;}manifest.status=manifest.results.every(r=>r.status==='passed')?'passed':'failed';}
  catch(e){manifest.status='failed';manifest.error=String(e.stack||e);}
  finally{const errors=[];for(const close of [()=>browser?.close(),()=>browserServer?.close()])try{await within(Promise.resolve(close()),5000,'browser cleanup');}catch(e){errors.push(String(e));}try{await stopProcess(browserServer?.process());}catch(e){errors.push(String(e));}if(server){server.closeAllConnections();try{await within(new Promise(r=>server.close(r)),3000,'HTTP cleanup');}catch(e){errors.push(String(e));}}if(errors.length){manifest.status='failed';manifest.cleanupErrors=errors;}await save();}
  return manifest;
}
if(process.argv[1]&&pathToFileURL(path.resolve(process.argv[1])).href===import.meta.url)main().then(r=>process.exit(r.status==='passed'?0:1),e=>{console.error(e);process.exit(1);});
