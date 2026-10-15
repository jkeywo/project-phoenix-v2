import {parse, stringify} from 'smol-toml';
import {entityInventory} from './workshop-entity-composition.js';
import {resolveTemplate} from './entity-includes.js';
import {authoredAndGeneratedSystems} from './workshop-generated-systems.js';
import {acceptWorkshopChanges} from './workshop-acceptance.js';

const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const closed = (value, keys) => object(value) && Object.keys(value).every(key => keys.includes(key));
const id = value => typeof value === 'string' && value.trim().length > 0;
const same = (a,b) => stable(a) === stable(b);
function stable(value) { return JSON.stringify(value,(_,row)=>object(row) ? Object.fromEntries(Object.keys(row).sort().map(key=>[key,row[key]])) : row); }
export const activityWorlds = draft => (draft?.paths() || []).filter(path=>path.startsWith('assets/worlds/') && path.endsWith('.toml') && !draft.isBinary(path))
  .filter(path=>{try{return !object(parse(draft.read(path)).domain);}catch{return true;}});

// Only the immutable equipment roots used by this form. Instance overrides
// replace arrays; doctrine's separate keyed merge is outside this projection.
function equipment(base, overrides) {
  const merge = (a,b) => object(a) && object(b) ? {...a,...Object.fromEntries(Object.entries(b).map(([key,value])=>[key,merge(a[key],value)]))} : structuredClone(b);
  const keys = ['tags','repair','interior','damage','system','console','station','shield_arc','logistics','infrastructure','comms'];
  const take = row => Object.fromEntries(keys.filter(key=>Object.hasOwn(row || {},key)).map(key=>[key,row[key]]));
  return merge(take(base),take(overrides));
}
export function inspectActivities(draft, dependencies, path) {
  const world = parse(draft.read(path));
  const sources = Object.fromEntries(entityInventory(draft, dependencies).map(row=>[row.path,row.text]));
  const resolve = (path, overrides) => { const result=resolveTemplate(path,sources,parse); if(!result.ok) throw result.error; return equipment(result.resolved.value,overrides); };
  const actors = [],rows=(world.entity || []).map(row=>({row,profile:resolve(row.template_path,row.overrides)}));
  const shipRows=rows.filter(({row,profile})=>row.spawn_on==='game_start' && profile.tags?.includes('ship'));
  const slots = world.ship_slot?.length ? world.ship_slot : (world.available_ships?.length ? [{id:'player',ships:world.available_ships}] : []);
  const offered=(slot,row)=>(slot.ships || []).map(ship=>resolve(ship.template_path,row?.overrides));
  for (const {row,profile} of rows) if(id(row.name)) {
    const index=shipRows.findIndex(value=>value.row===row),slot=slots[index];
    actors.push({id:row.name,profiles:slot?offered(slot,row):[profile]});
  }
  for (const [index,row] of slots.entries()) actors.push({id:'slot:'+row.id,profiles:shipRows[index]?offered(row,shipRows[index].row):[]});
  return {path,world,actors,definitions:structuredClone(world.situation_definition || []),situations:structuredClone(world.situation || [])};
}

export function activityChoices(profile) {
  const systems=authoredAndGeneratedSystems(profile), targets=[];
  for(const room of profile.interior?.compartment || []) targets.push({target:{kind:'compartment',compartment:room.id},goals:['structure']});
  for(const system of systems) for(const part of system.damage_component || []) targets.push({target:{kind:'component',system:system.id,component:part.id},
    goals:[...(part.patch_remedy?['patch']:[]),...(part.full_remedy?['full']:[])]});
  const deliveries=[];
  for(const handling of profile.logistics?.handling || []) for(const to of profile.logistics?.store || []) {
    const from=(profile.logistics?.store || []).find(row=>row.id===to.source);
    if(from?.role==='main' && to.role==='local' && from.resource===to.resource) deliveries.push({owner:handling.system,source:from.id,destination:to.id,commodity:to.resource,max:handling.max_batch});
  }
  const communications=(profile.comms?.radio?.channel || []).map(row=>({owner:profile.comms.radio.system,channel:row.id}));
  return {systems,targets,deliveries,communications};
}
function supports(request, profile) {
  const {systems,targets,deliveries,communications}=activityChoices(profile);
  if(request.kind==='communication') return request.request==='report_status' && systems.some(row=>row.id===request.owner && row.kind==='comms') && communications.some(row=>row.owner===request.owner && row.channel===request.channel);
  if(request.kind==='delivery') return deliveries.some(row=>row.owner===request.owner && row.source===request.source && row.destination===request.destination && row.commodity===request.commodity && request.amount<=row.max);
  if(!profile.damage || !systems.some(row=>row.id===request.owner && row.kind==='repair') || !Number.isInteger(request.team) || request.team<0 || request.team>255 || request.team>=profile.repair?.repair_team_count
    || !targets.some(row=>same(row.target,request.target) && row.goals.includes(request.goal))) return false;
  const visit=request.request, target=request.target, placements=profile.interior?.placements || {};
  if(visit?.type==='Work') return same(visit.data,target);
  const group = system => { const console=(profile.console || []).find(row=>row.id===system.console); return console?.home_station ? {type:'Station',data:console.home_station}
    : system.console ? {type:'Console',data:system.console} : {type:'Core'}; };
  // DamageDefinition keeps every installed System (including an empty
  // component map), and the ordinary visit resolver walks its BTreeMap keys.
  const ordered=[...systems].sort((a,b)=>a.id<b.id?-1:a.id>b.id?1:0);
  const selected=ordered.find(row=>visit?.type==='System' ? row.id===visit.data : same(group(row),visit));
  const room=target.kind==='compartment'?target.compartment:placements[target.system];
  return !!selected && !!room && placements[selected.id]===room;
}
export function validateActivities(definitions,situations,actors) {
  if(!Array.isArray(definitions) || definitions.length>32 || !Array.isArray(situations) || situations.length>32) throw Error('At most 32 declared situations and definitions are supported.');
  const unique=rows=>rows.every(row=>id(row.id)) && new Set(rows.map(row=>row.id)).size===rows.length;
  if(!unique(definitions)||!unique(situations)) throw Error('Use distinct nonempty definition and instance identifiers.');
  for(const def of definitions) {
    if(!closed(def,['id','initial','state']) || !Array.isArray(def.state) || !def.state.length || def.state.length>32 || !unique(def.state)) throw Error('Each definition requires a bounded named state chain.');
    const seen=new Set();let cursor=def.initial;
    while(cursor!=null) {
      const state=def.state.find(row=>row.id===cursor);
      if(!state || seen.has(cursor) || !closed(state,['id','when','call','next']) || !closed(state.when,['kind','outcome','state'])
        || !['accepted','achieved','settled','received','returned','communication_read','communication_reported','communication_declined','scenario_applied'].includes(state.when.kind)
        || (state.when.kind==='scenario_applied' ? !seen.has(state.when.state) || !def.state.some(row=>row.id===state.when.state && row.call) : Object.hasOwn(state.when,'state'))
        || (state.when.kind==='settled' ? !['completed','cancelled','failed'].includes(state.when.outcome) : Object.hasOwn(state.when,'outcome'))
        || (state.call!=null && (!closed(state.call,['script','function']) || !id(state.call.script) || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(state.call.function)))) throw Error('Use an acyclic exact evidence condition and a named Scenario function.');
      seen.add(cursor);cursor=state.next;
    }
    if(seen.size!==def.state.length) throw Error('Every state must be reachable from the initial state.');
  }
  for(let i=0;i<situations.length;i++) {
    const row=situations[i], request=row.request, actor=actors.find(actor=>actor.id===request?.actor);
    if(!closed(row,['id','definition','request']) || !definitions.some(def=>def.id===row.definition) || !actor?.profiles.length || !id(request.owner)
      || !(request.kind==='repair' ? closed(request,['kind','actor','owner','team','request','target','goal']) : (request.kind==='delivery' && closed(request,['kind','actor','owner','source','destination','commodity','amount']) && Number.isSafeInteger(request.amount) && request.amount>0) || (request.kind==='communication' && closed(request,['kind','actor','owner','recipient','channel','request']) && !request.recipient?.startsWith('slot:') && request.recipient!==request.actor && actors.some(row=>row.id===request.recipient && row.profiles.length && row.profiles.every(profile=>profile.comms?.radio)) && request.request==='report_status'))
      || !actor.profiles.every(profile=>supports(request,profile))) throw Error('Bind an actual declared actor, owning System and supported physical request/goal on every offered hull.');
    for(const previous of situations.slice(0,i)) { const a=previous.request,b=request;
      if(a.actor===b.actor && a.owner===b.owner && a.kind===b.kind && (a.kind==='repair' ? a.team===b.team || same(a.target,b.target) : a.kind==='delivery' ? a.source===b.source && a.destination===b.destination : a.recipient===b.recipient && a.channel===b.channel && a.request===b.request)) throw Error('Activity requests must be disjoint.'); }
  }
}
function write(source,definitions,situations) {
  const before=parse(source);
  if(same(before.situation_definition || [],definitions)&&same(before.situation || [],situations))return source;
  const headers=[...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)];
  const spans=headers.map((row,index)=>({key:row[1].replace(/\]$/,'').trim(),start:row.index,end:headers[index+1]?.index ?? source.length}));
  for(const span of spans.filter(row=>['situation','situation_definition'].some(root=>row.key===root || row.key.startsWith(root+'.'))).reverse()) source=source.slice(0,span.start)+source.slice(span.end);
  const nl=source.includes('\r\n')?'\r\n':'\n';
  source+=(source.endsWith('\n')?nl:nl+nl)+stringify({...(definitions.length?{situation_definition:definitions}:{}),...(situations.length?{situation:situations}:{})}).replace(/\r?\n/g,nl);
  const after=parse(source),rest=row=>{const copy=structuredClone(row);delete copy.situation;delete copy.situation_definition;return copy;};
  if(!same(after.situation_definition || [],definitions)||!same(after.situation || [],situations)||!same(rest(before),rest(after))) throw Error('Expand activity source into ordinary tables; unrelated World fields must remain unchanged.');
  return source;
}
export async function applyActivities({draft,provider,runtime,dependencies,path,definitions,situations,current=()=>true}) {
  const value=structuredClone({definitions,situations});
  return acceptWorkshopChanges({draft,provider,runtime,dependencies,current,prepare:(captured,effective)=>{
    const inspection=inspectActivities(captured,effective,path);validateActivities(value.definitions,value.situations,inspection.actors);
    const before=captured.read(path),after=write(before,value.definitions,value.situations);
    return before===after?[]:[{path,before,after}];
  }});
}
