// @vitest-environment jsdom
import {describe,it,expect,vi,afterEach} from 'vitest';
import {parse,stringify} from 'smol-toml';
import {WorkshopDocument} from '../../editor/workshop-document.js';
import {createStoreZip} from '../../editor/mod-pack-export.js';
import {inspectActivities,validateActivities,applyActivities} from '../../editor/workshop-activities.js';
import {mountWorkshopActivities} from '../../gui/workshop-activities-panel.js';
import {createWorkshopOperations} from '../../gui/workshop-edit-session.js';
const path='assets/worlds/activity.toml',hull='assets/entities/activity.toml';
const profile=()=>({tags:['ship'],system:[{id:'repair',kind:'repair',console:'engineering'},{id:'handling',kind:'logistics',console:'engineering'},
  {id:'engine',kind:'helm_thrust',console:'engineering',damage_component:[{id:'operation',full_remedy:'full',patch_remedy:'patch'}]}],
  console:[{id:'engineering',home_station:'engineering'}],station:[{id:'engineering'}],repair:{repair_team_count:2},damage:{},
  interior:{compartment:[{id:'engineering'}],placements:{repair:'engineering',engine:'engineering'}},
  logistics:{handling:[{system:'handling',max_batch:4}],store:[{id:'hold',role:'main',resource:'parts'},{id:'local',role:'local',resource:'parts',source:'hold'}]}});
const defs=()=>[{id:'work',initial:'useful',state:[{id:'useful',when:{kind:'achieved'},call:{script:'assets/worlds/activity.toml#script.setup',function:'credit'},next:'settled'},
  {id:'settled',when:{kind:'settled',outcome:'cancelled'}}]}];
const situations=()=>[{id:'restore',definition:'work',request:{kind:'repair',actor:'ship',owner:'repair',team:0,
  request:{type:'Work',data:{kind:'component',system:'engine',component:'operation'}},target:{kind:'component',system:'engine',component:'operation'},goal:'full'}}];
const source=()=>stringify({global:{seed:22},entity:[{template_path:hull,name:'ship',spawn_on:'game_start'}],script:{setup:'fn credit(ctx) { ctx.flags.increment("credit", 1); }'},situation_definition:defs(),situation:situations()});
const deps=()=>({base_files:{[hull]:stringify(profile())},packs:[]});
const documentFor=text=>new WorkshopDocument(createStoreZip([{path:'scenarios.toml',text:'[pack]\nformat=1\nid="activity"\nname="Activity"\nversion="1"\n'},{path,text}]));
const settle=async()=>{for(let n=0;n<20;n++)await new Promise(resolve=>setTimeout(resolve,1));};
afterEach(()=>document.body.replaceChildren());
describe('accepted physical situation source',()=>{
  it('keeps shared definitions and disjoint instances in chronological accepted history/export',async()=>{
    const draft=documentFor(source()),dependencies=deps(),before=draft.read(path),inspection=inspectActivities(draft,dependencies,path);
    const rows=situations();rows.push({id:'supply',definition:'work',request:{kind:'delivery',actor:'ship',owner:'handling',source:'hold',destination:'local',commodity:'parts',amount:2}});
    validateActivities(defs(),rows,inspection.actors);
    const runtime={validate:vi.fn(async(_,candidate)=>({accepted:parse(candidate.read(path)).situation.length===2}))};
    await applyActivities({draft,dependencies,runtime,path,definitions:defs(),situations:rows});
    expect(runtime.validate).toHaveBeenCalledOnce();expect(parse(draft.read(path)).script).toEqual(parse(before).script);
    expect(inspectActivities(new WorkshopDocument(draft.archive()),dependencies,path).situations).toEqual(rows);
    draft.undo();expect(draft.read(path)).toBe(before);draft.redo();expect(parse(draft.read(path)).situation).toEqual(rows);
    await applyActivities({draft,dependencies,runtime:{validate:async()=>({accepted:true})},path,definitions:[],situations:[]});
    expect(inspectActivities(draft,dependencies,path).situations).toEqual([]);expect(parse(draft.read(path)).script).toEqual(parse(before).script);
  });
  it('refuses dynamic actors, unsupported actual requests, overlaps, hidden keys and cycles before acceptance',async()=>{
    const draft=documentFor(source()),dependencies=deps(),inspection=inspectActivities(draft,dependencies,path);
    for(const edit of [row=>row.request.actor='dynamic',row=>row.request.team=2,row=>row.request.team=256,row=>row.request.owner='foreign',row=>row.request.target.component='missing',row=>row.request.request={type:'External'},row=>row.request.goal='magic',row=>row.request.extra=true]){
      const rows=situations();edit(rows[0]);expect(()=>validateActivities(defs(),rows,inspection.actors)).toThrow();
    }
    const duplicate=situations();duplicate.push({...structuredClone(duplicate[0]),id:'overlap'});expect(()=>validateActivities(defs(),duplicate,inspection.actors)).toThrow(/disjoint/);
    const cyclic=defs();cyclic[0].state[1].next='useful';expect(()=>validateActivities(cyclic,situations(),inspection.actors)).toThrow();
    const changed=situations();changed[0].id='changed';
    const before=draft.snapshot();await expect(applyActivities({draft,dependencies,runtime:{validate:async()=>({accepted:false})},path,definitions:defs(),situations:changed})).rejects.toThrow();expect(draft.snapshot()).toEqual(before);
  });
  it('checks every actual offered slot hull and applies supported instance array replacement',()=>{
    const dependencies=deps(),other='assets/entities/other.toml';dependencies.base_files[other]=stringify({...profile(),repair:{repair_team_count:0}});
    const world=parse(source());world.ship_slot=[{id:'lead',ships:[{template_path:hull},{template_path:other}],default_ship:hull}];
    const draft=documentFor(stringify(world)),inspection=inspectActivities(draft,dependencies,path);
    const rows=situations();rows[0].request.actor='slot:lead';expect(()=>validateActivities(defs(),rows,inspection.actors)).toThrow();
    world.entity[0].overrides={repair:{repair_team_count:1}};
    rows[0].request.actor='ship';rows[0].request.team=1;expect(()=>validateActivities(defs(),rows,inspectActivities(documentFor(stringify(world)),dependencies,path).actors)).toThrow();
  });
  it('matches ordinary sorted visit rooms even when the first System has no damage components',()=>{
    const value=profile(),engine=value.system.find(row=>row.id==='engine');engine.id='z-engine';
    value.system=[engine,{id:'a-router',kind:'command',console:'engineering'},...value.system.filter(row=>row!==engine)];
    value.interior.compartment.push({id:'support'});value.interior.placements={'a-router':'support','z-engine':'engineering',repair:'engineering',handling:'engineering'};
    const rows=situations();rows[0].request.request={type:'Station',data:'engineering'};
    rows[0].request.target={kind:'component',system:'z-engine',component:'operation'};
    const actors=[{id:'ship',profiles:[value]}];
    expect(()=>validateActivities(defs(),rows,actors)).toThrow();
    rows[0].request.target={kind:'compartment',compartment:'support'};rows[0].request.goal='structure';
    expect(()=>validateActivities(defs(),rows,actors)).not.toThrow();
    value.system.reverse();expect(()=>validateActivities(defs(),rows,actors)).not.toThrow();
  });
  it('does not rewrite misleading script headers or accept a stale async candidate',async()=>{
    const draft=documentFor(source()),dependencies=deps();let finish;const runtime={validate:()=>new Promise(resolve=>{finish=resolve;})};
    const changed=situations();changed[0].id='changed';
    const pending=applyActivities({draft,dependencies,runtime,path,definitions:defs(),situations:changed});
    await settle();draft.apply([{path,before:draft.read(path),after:draft.read(path)+'\n# concurrent\n'}]);finish({accepted:true});await expect(pending).rejects.toThrow(/stale/);
    const misleading=documentFor(source().replace(/^setup = .*$/m,"setup = '''\nfn credit(ctx) {}\n/*\n[[situation]]\n*/\n'''"));
    const before=misleading.snapshot();await expect(applyActivities({draft:misleading,dependencies,runtime:{validate:async()=>({accepted:true})},path,definitions:defs(),situations:changed})).rejects.toThrow();expect(misleading.snapshot()).toEqual(before);
  });
});
describe('structured activity Workshop controls',()=>{
  it('preserves focus, accepts exact fields and replaces a same-revision document safely',async()=>{
    document.body.innerHTML='<main></main>';let draft=documentFor(source()),busy=false;
    const runtime={dependencies:async()=>deps(),validate:vi.fn(async()=>({accepted:true}))};
    const panel=mountWorkshopActivities({root:document.querySelector('main'),runtime,draft:()=>draft,busy:()=>busy,setBusy:value=>{busy=value;}});await settle();
    const name=panel.node.querySelector('input[id*="instance-"]');name.value='edited';name.focus();panel.refresh();expect(document.activeElement).toBe(name);expect(name.value).toBe('edited');
    panel.node.querySelector('#workshop-activities-save').click();await settle();expect(parse(draft.read(path)).situation[0].id).toBe('edited');
    const replacement=parse(source());replacement.situation[0].id='other-pack';draft=documentFor(stringify(replacement));panel.refresh();
    expect(panel.node.querySelector('input[id*="instance-"]').value).toBe('other-pack');panel.node.querySelector('#workshop-activities-save').click();await settle();expect(parse(draft.read(path)).situation[0].id).toBe('other-pack');panel.dispose();
  });
  it('discards accepted async edits after a document swap or disposal',async()=>{
    document.body.innerHTML='<main></main>';let draft=documentFor(source()),finish;const operations=createWorkshopOperations();
    const runtime={dependencies:async()=>deps(),validate:()=>new Promise(resolve=>{finish=resolve;})};
    const panel=mountWorkshopActivities({root:document.querySelector('main'),runtime,draft:()=>draft,...operations.panel()});await settle();
    panel.node.querySelector('input[id*="instance-"]').value='must-not-commit';const original=draft,before=original.snapshot();panel.node.querySelector('#workshop-activities-save').click();await settle();
    expect(operations.busy()).toBe(true);draft=documentFor(source());panel.dispose();finish({accepted:true});await settle();expect(original.snapshot()).toEqual(before);expect(operations.busy()).toBe(false);
  });
});

describe('communication activity authoring',()=>{
  it('accepts only actual supported radio owners and declared base recipients through validated history',async()=>{
    const value=profile();value.system.push({id:'comms',kind:'comms',console:'engineering'});value.comms={radio:{system:'comms',request_reading_ticks:30,channel:[{id:'public',range:200,work_units:1}]}};
    const dependencies={base_files:{[hull]:stringify(value)},packs:[]};
    const world=parse(source());world.entity.push({template_path:hull,name:'port',spawn_on:'game_start'});
    const draft=documentFor(stringify(world));
    const definitions=[{id:'report',initial:'heard',state:[{id:'heard',when:{kind:'communication_reported'}}]}];
    const rows=[{id:'contact',definition:'report',request:{kind:'communication',actor:'ship',owner:'comms',recipient:'port',channel:'public',request:'report_status'}}];
    const inspection=inspectActivities(draft,dependencies,path);validateActivities(definitions,rows,inspection.actors);
    for(const edit of [row=>row.request.recipient='slot:player',row=>row.request.recipient='unknown',row=>row.request.owner='repair',row=>row.request.channel='private',row=>row.request.request='order_attack']){
      const candidate=structuredClone(rows);edit(candidate[0]);expect(()=>validateActivities(definitions,candidate,inspection.actors)).toThrow();
    }
    const before=draft.toFiles(),runtime={validate:vi.fn(async()=>({accepted:true}))};
    await applyActivities({draft,dependencies,runtime,path,definitions,situations:rows});
    expect(runtime.validate).toHaveBeenCalledOnce();
    expect(parse(new WorkshopDocument(draft.archive()).read(path)).situation).toEqual(rows);
    draft.undo();expect(draft.toFiles()).toEqual(before);draft.redo();expect(parse(draft.read(path)).situation[0].request).toEqual(rows[0].request);
  });
});
