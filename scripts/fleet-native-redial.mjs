// Bounded all-native GM socket recovery. Only the transport socket is closed.
import { nativeRecoveryPeer } from './fleet-mixed-recovery.mjs';
const labels=['ship-1','ship-2','ship-3','ship-4','gm-1','gm-2'];
const positive=value=>Number.isSafeInteger(value)&&value>0;
const last=(rows,kind)=>rows.filter(row=>row.kind===kind).at(-1)?.value;
export function nativeGmRedialOutcome(e) {
  const reasons=[],need=(ok,message)=>{if(!ok)reasons.push(message);};
  const before=e.before||[],after=e.after||[],target=before.find(peer=>peer.label==='gm-2');
  need(before.length===6&&after.length===6&&new Set(after.map(peer=>peer.slot)).size===6
    &&before.every(peer=>peer.adopted&&after.some(row=>row.label===peer.label&&row.slot===peer.slot&&row.adopted)),
    'six unchanged admitted peers required');
  need(positive(e.processBefore?.pid)&&e.processBefore.alive===true&&e.processAfter?.alive===true
    &&e.processAfter.pid===e.processBefore.pid&&e.processAfter.startedAt===e.processBefore.startedAt,
    'same live native process required');
  const trigger=e.trigger;
  need(Number.isSafeInteger(trigger?.generation)&&trigger.generation>=0&&trigger.memberCreations===1
    &&e.closed?.generation===trigger.generation,'one existing member socket must close');
  need(positive(e.opened?.generation)&&e.opened.generation>trigger?.generation
    &&e.opened.event==='open'&&e.openRequest?.generation===e.opened.generation,
    'new native transport generation must actually open');
  need(e.identity?.sameCredential===true&&e.identity.sameOperator===true&&e.identity.sameSlot===true,
    'private reconnect capability and identity must be preserved');
  const commit=e.commits?.[0];
  need(positive(commit?.id)&&commit?.kind==='reconnect'&&commit.candidate?.host===target?.slot
    &&positive(commit.tick)&&e.commits.length===6&&e.commits.every(row=>JSON.stringify(row)===JSON.stringify(commit)),
    'six authoritative matching reconnect commits required');
  const losses=after.filter(peer=>peer.label!=='gm-2').map(peer=>peer.recovery?.losses?.filter(row=>row.slot===target?.slot)||[]);
  const lossTick=losses[0]?.[0]?.tick;
  need(losses.length===5&&positive(lossTick)&&losses.every(rows=>rows.length===1&&rows[0].tick===lossTick)
    &&lossTick<=commit?.tick,'five survivors must agree one prior HostLoss');
  need(e.resume?.accepted===true&&e.resumeApplied===true,'explicit post-commit Resume must apply');
  const ticks=new Map();let malformed=false,contradictory=false;
  for(const peer of after)for(const frame of peer.frames||[]) {
    const d=frame.d;if(frame.t!=='digest'||d.tick<=Math.max(e.atTick||0,commit?.tick||Infinity))continue;
    if(d.from!==peer.slot||!positive(d.tick)||typeof d.digest!=='string'||!/^[0-9a-f]{16}$/.test(d.digest)){malformed=true;continue;}
    if(!ticks.has(d.tick))ticks.set(d.tick,new Map());const rows=ticks.get(d.tick);
    if(rows.has(peer.label)&&rows.get(peer.label)!==d.digest)contradictory=true;
    rows.set(peer.label,d.digest);
  }
  const common=[...ticks].filter(([,rows])=>rows.size===6).map(([tick,rows])=>({tick,byPeer:Object.fromEntries(rows),agreed:new Set(rows.values()).size===1}));
  need(!malformed&&!contradictory&&common.length>=2&&common.every(row=>row.agreed),'two exact post-commit digest checkpoints required');
  const duplicates=[];
  for(const peer of after){const seen=new Set();for(const command of peer.commands||[]){const key=JSON.stringify([command.origin,command.seq]);if(seen.has(key))duplicates.push({peer:peer.label,key});seen.add(key);}}
  need(!duplicates.length,'duplicate outgoing command orders');
  need(before.filter(peer=>peer.label.startsWith('ship-')).every(peer=>{
    const ids=new Set((peer.commands||[]).map(row=>row.ship));
    return ids.size===1&&after.find(row=>row.label===peer.label)?.commands?.some(row=>row.tick>commit?.tick&&ids.has(row.ship));
  }),'original ship identities must remain in fresh controls');
  need(after.every(peer=>!peer.errors?.length),'runtime observer failure');
  return {passed:!reasons.length,reasons,lossTick,commit,commonDigests:common,duplicateOrders:duplicates};
}
export function nativeGmRedialHook({faultSeconds=600}={}) {
  if(!positive(faultSeconds)||faultSeconds>600)throw new Error('Invalid redial deadline');
  return async({result,step,wait,deadline,commandNative,commandGm,nativeProcess})=>{
    const capture=()=>labels.map(label=>nativeRecoveryPeer(label,result.nativeEvents[label]||[]));
    const before=capture(),indexes=Object.fromEntries(labels.map(label=>[label,result.nativeEvents[label]?.length||0]));
    const rows=label=>(result.nativeEvents[label]||[]).slice(indexes[label]);
    const e=result.recovery={failure:'gm-redial',before,processBefore:nativeProcess('gm-2'),
      atTick:Math.max(...before.flatMap(peer=>peer.frames.map(frame=>frame.d.tick))),samples:[]};
    const faultDeadline=Math.min(deadline,Date.now()+faultSeconds*1000),correlation='1534-native-redial-resume';
    commandNative('gm-2',{kind:'gm-redial'});step('close existing native GM socket without replacing its process');
    let resumeSent=false;
    await wait(()=>{
      e.after=capture();e.processAfter=nativeProcess('gm-2');
      const targetRows=rows('gm-2');
      e.trigger=last(targetRows,'redial-requested');e.closed=last(targetRows,'fleet_wire_close');
      e.opened=targetRows.filter(row=>row.kind==='fleet-wire-event'&&row.value.event==='open').at(-1)?.value;
      e.openRequest=last(targetRows,'fleet_wire_open');e.identity=last(targetRows,'redial-identity');
      e.commits=labels.map(label=>rows(label).filter(row=>row.kind==='state'&&row.value.gm_join?.status==='committed').at(-1)?.value.gm_join.commit);
      if(!resumeSent&&e.commits.every(commit=>commit?.kind==='reconnect')){
        commandGm('gm-1',{kind:'resume',correlation});resumeSent=true;
      }
      e.resume=last(rows('gm-1'),'gm-resume-requested');
      e.resumeApplied=rows('gm-1').some(row=>row.kind==='gm-activity'&&row.value.entries?.some(entry=>
        entry.detail?.type==='gm_action'&&entry.detail.data.correlation===correlation&&entry.detail.data.outcome==='applied'));
      e.outcome=nativeGmRedialOutcome(e);
      if(e.samples.length>=6000)throw new Error('Native redial sample overflow');
      e.samples.push({at:new Date().toISOString(),outcome:e.outcome});
      if(Date.now()>faultDeadline)throw new Error('Native redial deadline: '+e.outcome.reasons.join('; '));
      return e.outcome.passed;
    },'same native GM reconnects through canonical restore and explicit Resume');
    step('native GM redial preserved process and capability with two matching fleet digests');
  };
}
