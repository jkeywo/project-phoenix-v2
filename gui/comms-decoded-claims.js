import {t} from './strings.js';
const clock=value=>Number.isSafeInteger(value)&&value>=0;
const shape=(value,keys)=>value&&typeof value==='object'&&!Array.isArray(value)
  &&Object.keys(value).sort().join('|')===[...keys].sort().join('|');
function requestClaim(value){
  if(!value)return null;
  if(['report_status','receive_craft'].includes(value.kind)&&shape(value,['kind']))return {kind:value.kind};
  if(value.kind==='form_up'&&shape(value,['kind','slot'])&&Array.isArray(value.slot)&&value.slot.length===2
    &&value.slot.every(axis=>Number.isInteger(axis)&&Math.abs(axis)<=1000000))return {kind:value.kind,slot:[...value.slot]};
  if(value.kind==='media_call'&&shape(value,['kind','mode'])&&['audio','audio_video'].includes(value.mode))return {kind:value.kind,mode:value.mode};
  return null;
}
/** Closed evidence only. Never feed this value into addressed request helpers. */
export function decodedClaim(value,kind,emitted){
  if(!clock(emitted)||!value||value.kind!==kind)return null;
  if(kind==='text')return shape(value,['kind'])?{kind}:null;
  if(kind==='request'){
    const request=requestClaim(value.request);
    return shape(value,['kind','request'])&&request?{kind,request}:null;
  }
  if(kind!=='outcome'||!shape(value,['kind','outcome']))return null;
  const data=value.outcome;
  if(!shape(data,['request','request_emitted_tick','heard_tick','read_tick','decision','report']))return null;
  const request=requestClaim(data.request);
  if(!request||!clock(data.request_emitted_tick)||!clock(data.heard_tick)
    ||data.request_emitted_tick>data.heard_tick||data.heard_tick>emitted
    ||data.read_tick!==null&&(!clock(data.read_tick)||data.read_tick<data.heard_tick||data.read_tick>emitted))return null;
  let decision=null,report=null;
  if(data.decision!==null){
    const item=data.decision;
    if(!shape(item,['choice','tick'])||!['accept','decline','report'].includes(item.choice)
      ||data.read_tick===null||!clock(item.tick)||item.tick<data.read_tick||item.tick>emitted
      ||item.choice==='report'&&request.kind!=='report_status'
      ||item.choice==='accept'&&request.kind==='report_status')return null;
    decision={choice:item.choice,tick:item.tick};
  }
  if(data.report!==null){
    const item=data.report;
    if(!shape(item,['available','observed_tick'])||typeof item.available!=='boolean'
      ||decision?.choice!=='report'||item.observed_tick!==decision.tick)return null;
    report={available:item.available,observed_tick:item.observed_tick};
  }
  if((decision?.choice==='report')!==(report!==null))return null;
  return {kind,outcome:{request,request_emitted_tick:data.request_emitted_tick,heard_tick:data.heard_tick,
    read_tick:data.read_tick,decision,report}};
}
function requestText(request){
  if(request.kind==='form_up')return t('comms.request.form_up')+' · '+t('comms.request.formation_slot',{x:request.slot[0],z:request.slot[1]});
  if(request.kind==='media_call')return t(request.mode==='audio'?'comms.request.media_audio':'comms.request.media_video');
  return t(request.kind==='receive_craft'?'comms.capture.receive_craft':'comms.request.report_status');
}
/** Label all foreign events as an attributed claim, never this Console's receipt. */
export function decodedClaimText(claim){
  if(!claim||claim.kind==='text')return '';
  if(claim.kind==='request')return t('comms.capture.typed_claim',{content:requestText(claim.request)});
  const data=claim.outcome,parts=[requestText(data.request),
    t('comms.capture.request_emitted_at',{tick:data.request_emitted_tick}),t('comms.request.heard_at',{tick:data.heard_tick})];
  if(data.read_tick!==null)parts.push(t('comms.request.read_at',{tick:data.read_tick}));
  if(data.decision)parts.push(t(data.decision.choice==='report'?'comms.request.reported_at'
    :data.decision.choice==='accept'?'comms.request.accepted_at':'comms.request.declined_at',{tick:data.decision.tick}));
  if(data.report)parts.push(t(data.report.available?'comms.request.available_at':'comms.request.unavailable_at',{tick:data.report.observed_tick}));
  return t('comms.capture.typed_claim',{content:parts.join(' · ')});
}
