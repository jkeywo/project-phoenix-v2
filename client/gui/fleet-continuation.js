// Runtime continuation for one departed star owner. These counters describe
// transport delivery, never simulation time, command order, or ship identity.
export const CONTINUATION_LIMITS = Object.freeze({frames:4096,bytes:8*1024*1024});
const ordinal = value => Number.isSafeInteger(value) && value > 0;
const sequence = value => Number.isSafeInteger(value) && value >= 0;
const keyOf = row => `${row.origin}:${row.sequence}`;
const rowBytes = row => row.raw.length * 2 + 32; // conservative UTF-16 bound
const clone = value => JSON.parse(JSON.stringify(value));
function membersOf(values) {
  if (!Array.isArray(values) || values.length < 2 || values.length > 32 || values.some(value=>!ordinal(value)) || new Set(values).size !== values.length) throw new Error('invalid-continuation-participants');
  return [...values].sort((a,b)=>a-b);
}
function validVector(vector,members) {
  return vector && typeof vector === 'object' && !Array.isArray(vector)
    && Object.keys(vector).length === members.length && members.every(slot=>sequence(vector[slot]));
}
function validateRow(row,members) {
  if (!row || !members.includes(row.origin) || !ordinal(row.sequence) || typeof row.raw !== 'string' || !row.raw.length) throw new Error('invalid-continuation-row');
}

/** Retains exactly the part of each reliable stream not acknowledged by all. */
export function createContinuationJournal({local,participants,limits=CONTINUATION_LIMITS}) {
  let members=membersOf(participants);
  if (!members.includes(local) || !ordinal(limits.frames) || !ordinal(limits.bytes)) throw new Error('invalid-continuation-journal');
  const seen=Object.fromEntries(members.map(slot=>[slot,0]));
  const acknowledgements=new Map(members.map(slot=>[slot,{...seen}]));
  const rows=new Map();
  let bytes=0,failed=null,holding=false;
  let active=[...members];
  const refuse=reason=>{failed=reason;throw new Error(reason);};
  const live=()=>{if(failed)throw new Error(failed);};
  function retain(row) {
    const size=rowBytes(row);
    if(rows.size+1>limits.frames || bytes+size>limits.bytes) refuse('continuation-tail-overflow');
    rows.set(keyOf(row),row);bytes+=size;
  }
  function compact() {
    if(holding)return;
    for(const [key,row] of rows) {
      const acknowledged=active.every(slot=>acknowledgements.get(slot)[row.origin]>=row.sequence);
      if(acknowledged){rows.delete(key);bytes-=rowBytes(row);}
    }
  }
  function record(raw) {
    live();
    if(holding)throw new Error('continuation-stream-held');
    const row={origin:local,sequence:seen[local]+1,raw};
    validateRow(row,members);retain(row);
    seen[local]=row.sequence;acknowledgements.set(local,{...seen});
    return {...row,ack:{...seen}};
  }
  function receive(envelope,authenticatedOrigin) {
    live();validateRow(envelope,members);
    if(authenticatedOrigin!==envelope.origin)throw new Error('continuation-origin-mismatch');
    if (!envelope.ack || Object.keys(envelope.ack).some(slot => !members.includes(Number(slot)))
        || Object.values(envelope.ack).some(value => !sequence(value))) throw new Error('invalid-continuation-ack');
    const incomingAck = Object.fromEntries(members.map(slot => [slot, envelope.ack[slot] || 0]));
    const row={origin:envelope.origin,sequence:envelope.sequence,raw:envelope.raw};
    const previous=rows.get(keyOf(row));
    if(previous && previous.raw!==row.raw)refuse('conflicting-continuation-frame');
    if(row.sequence>seen[row.origin]+1)refuse('continuation-stream-gap');
    const fresh=row.sequence>seen[row.origin];
    if(fresh){retain(row);seen[row.origin]=row.sequence;}
    const oldAck=acknowledgements.get(row.origin);
    acknowledgements.set(row.origin,Object.fromEntries(members.map(slot=>[slot,Math.max(oldAck[slot],incomingAck[slot])])));
    acknowledgements.set(local,{...seen});compact();
    return fresh?row.raw:null;
  }
  return {
    record,receive,
    frontier(){return {...seen};},
    sync(participants, baseline=null) {
      live();
      const next=membersOf([...new Set([...members,...participants])]);
      for(const slot of next) if(!members.includes(slot)) {
        seen[slot]=0;
        for(const ack of acknowledgements.values())ack[slot]=0;
        acknowledgements.set(slot,Object.fromEntries(next.map(origin=>[origin,0])));
      }
      members=next;active=[...participants];
      if(baseline) {
        if(rows.size || Object.values(seen).some(value=>value!==0) || !validVector(baseline,members))throw new Error('invalid-continuation-baseline');
        Object.assign(seen,baseline);
        acknowledgements.set(local,{...seen});
      }
      compact();
    },
    hold(){live();holding=true;return this.tail();},
    tail(){live();return {local,participants:[...members],seen:{...seen},rows:[...rows.values()].map(clone)};},
    /** Replays only unseen rows after a fully validated survivor union. */
    replay(plan,deliver){
      live();if(!holding)throw new Error('continuation-not-held');
      const target=plan.targets?.find(target=>target.local===local);
      if(!target || JSON.stringify(target.before)!==JSON.stringify(seen))throw new Error('stale-continuation-plan');
      for(const row of target.rows){
        if(row.sequence!==seen[row.origin]+1)refuse('continuation-stream-gap');
        retain(clone(row));deliver(row.raw,row.origin);seen[row.origin]=row.sequence;
      }
      if (!validVector(plan.frontier,members) || members.some(slot=>seen[slot]!==plan.frontier[slot])) refuse('invalid-continuation-frontier');
      acknowledgements.set(local,{...seen});
      return {...seen};
    },
    commit({departed,frontier}) {
      live();
      if(!holding || !active.includes(departed) || departed===local || !validVector(frontier,members)
        || members.some(slot=>seen[slot]!==frontier[slot]))throw new Error('invalid-continuation-commit');
      active=active.filter(slot=>slot!==departed);
      // Commit is sent only after every survivor confirms this exact frontier.
      for(const slot of active)acknowledgements.set(slot,{...frontier});
      holding=false;compact();
    },
    get failed(){return failed;},
    get retained(){return {frames:rows.size,bytes};},
  };
}

/** Every live participant must report its frozen delivery frontier. */
export function reconcileContinuation({participants,departed,tails,coordinator=Math.min(...participants.filter(slot=>slot!==departed)),limits=CONTINUATION_LIMITS}) {
  const members=membersOf(participants),survivors=members.filter(slot=>slot!==departed);
  if(!members.includes(departed) || !Array.isArray(tails) || tails.length!==survivors.length || new Set(tails.map(tail=>tail.local)).size!==survivors.length)throw new Error('incomplete-continuation-survivors');
  const rows=new Map(),authorizedRows=new Map(),frontier=Object.fromEntries(members.map(slot=>[slot,0]));
  let bytes=0;
  for(const tail of tails){
    if(!survivors.includes(tail.local) || JSON.stringify(tail.participants)!==JSON.stringify(members) || !validVector(tail.seen,members) || !Array.isArray(tail.rows))throw new Error('invalid-continuation-tail');
    for(const slot of members)frontier[slot]=Math.max(frontier[slot],tail.seen[slot]);
    for(const row of tail.rows){
      validateRow(row,members);
      if(row.sequence>tail.seen[row.origin])throw new Error('unseen-continuation-row');
      const key=keyOf(row),previous=rows.get(key);
      if(previous && previous.raw!==row.raw)throw new Error('conflicting-continuation-frame');
      if(!previous){rows.set(key,clone(row));bytes+=rowBytes(row);}
      // Preserve the existing star trust boundary. A survivor may prove its
      // own stream; only the elected successor's locally authenticated history
      // can prove a departed owner's row. A foreign carrier is not its origin.
      if(tail.local===(row.origin===departed?coordinator:row.origin))authorizedRows.set(key,clone(row));
      if(rows.size>limits.frames || bytes>limits.bytes)throw new Error('continuation-tail-overflow');
    }
  }
  const coordinatorTail=tails.find(tail=>tail.local===coordinator);
  if(!coordinatorTail || frontier[departed]>coordinatorTail.seen[departed])throw new Error('unverifiable-owner-suffix');
  const targets=tails.map(tail=>{
    const missing=[];
    for(const origin of members)for(let sequence=tail.seen[origin]+1;sequence<=frontier[origin];sequence++){
      // The frame bound also bounds adversarially large advertised gaps.
      if(missing.length>=limits.frames)throw new Error('continuation-tail-overflow');
      const row=authorizedRows.get(`${origin}:${sequence}`);
      if(!row)throw new Error(rows.has(`${origin}:${sequence}`)?'unverifiable-origin-suffix':'missing-continuation-frame');
      missing.push(row);
    }
    return {local:tail.local,before:{...tail.seen},rows:missing};
  });
  return {departed,frontier,targets};
}
