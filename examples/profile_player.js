// Real native participant, same console documents and ordinary station admission.
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const mark = (stage, extra={}) => fetch('MARKER_ORIGIN/'+encodeURIComponent(JSON.stringify({stage,...extra})));
const wait = async (fn, label) => { const end=Date.now()+45000; while(!fn()){if(Date.now()>end)throw Error('Timeout: '+label);await sleep(100);} };
// Isolate operator preferences even when Windows ignores APPDATA overrides.
let operatorProfile=null;
const drain=window.__phoenixPaneOutDrain;
window.__phoenixPaneOutDrain=function(){
  return drain().split('\n').filter(line=>{
    let request;try{request=JSON.parse(line);}catch{return true;}
    if(request.type!=='NativeOperator'||!['load','save'].includes(request.operation))return true;
    if(request.operation==='save')operatorProfile=request.profile;
    setTimeout(()=>window.__phoenixOperatorReply({operation:request.operation,status:'ok',profile:operatorProfile}),0);
    return false;
  }).join('\n');
};
let phase=null, assigned=null, updates=0, lastReading=0, changedReadings=0, previous=null;
let recording=false;
const intervals=[], durations=[], invalid=new Set();
const apply=window.__phoenixPaneApply;
window.__phoenixPaneApply=function(json){
  const message=JSON.parse(json);
  if(message.type==='Welcome') phase=message.data?.state?.phase;
  if(message.type==='LobbyState') phase=message.data?.phase;
  if(message.type==='GameStarted') phase='InProgress';
  if(message.type==='StationAssigned' && message.data?.token===window.__phoenixPane?.token) assigned=message.data.station;
  return apply.apply(this,arguments);
};
const stats = a => { const s=a.slice().sort((x,y)=>x-y);return {count:s.length,p50:s[Math.floor(s.length*.5)],p95:s[Math.floor(s.length*.95)],max:s.at(-1)}; };
try {
  await wait(()=>window.phoenixLink && window.simState?.hullId,'welcome');
  window.phoenixLink.send('SelectStation',{station:'helm'});
  await wait(()=>assigned==='helm','Helm admission');
  window.phoenixLink.send('SetReady',{ready:true});
  await wait(()=>document.getElementById('helm-iframe')?.contentWindow?.__updateConsole,'Helm mount');
  await wait(()=>document.getElementById('helm-iframe')?.getBoundingClientRect().height>0,'visible Helm');
  const frame=document.getElementById('helm-iframe'), mounted=frame.contentWindow;
  const update=mounted.__updateConsole;
  mounted.__updateConsole=function(name,json){
    const start=performance.now(); const result=update.apply(this,arguments);
    if(recording){updates++;durations.push(performance.now()-start);intervals.push(start-lastReading);if(json!==previous)changedReadings++;}
    previous=json;lastReading=start;return result;
  };
  const viewport=[innerWidth,innerHeight];
  await mark('workload',{viewport,station:'helm',hull:window.simState.hullId});
  await mark('PROFILE_STAGE-warmup');await sleep(40000);
  const health=setInterval(()=>{
    if(innerWidth!==viewport[0]||innerHeight!==viewport[1])invalid.add('resized');
    if(document.getElementById('helm-iframe')?.contentWindow!==mounted)invalid.add('console-remounted');
    if(performance.now()-lastReading>1000)invalid.add('stale-readings');
    if(assigned!=='helm')invalid.add('station-not-owned');
    if(phase!=='InProgress')invalid.add('phase');
    if(!frame.getBoundingClientRect().width||!frame.getBoundingClientRect().height)invalid.add('hidden-console');
  },250);
  recording=true;await mark('PROFILE_STAGE-start');await sleep(60000);recording=false;clearInterval(health);
  await mark('health',{invalid:[...invalid],changedReadings,consoleMs:stats(durations),readingIntervalMs:stats(intervals)});
  await mark('PROFILE_STAGE-end',{phase,updates,assigned,hull:window.simState.hullId,consoleSize:[frame.clientWidth,frame.clientHeight]});
  await mark('complete');
}catch(error){await mark('error',{message:String(error)});}
