// @vitest-environment jsdom
import {describe,it,expect,vi,afterEach} from 'vitest';
import {readFileSync} from 'node:fs';
import {parse,stringify} from 'smol-toml';
import {WorkshopDocument} from '../../editor/workshop-document.js';
import {createStoreZip} from '../../editor/mod-pack-export.js';
import {inspectPlanning,applyPlanning,planningWorlds} from '../../editor/workshop-planning.js';
import {mountWorkshopPlanning} from '../../gui/workshop-planning-panel.js';
import {activityWorlds} from '../../editor/workshop-activities.js';
import {createWorkshopOperations} from '../../gui/workshop-edit-session.js';
const path='assets/worlds/maintenance.toml',lib='assets/worlds/domains/maintenance.toml';
const library=readFileSync('assets/worlds/domains/maintenance.toml','utf8');
const world=()=>stringify({global:{seed:22},entity:[{template_path:'assets/entities/alliance_cruiser.toml',name:'ship'}],
  script:{setup:'fn unchanged(ctx) {}'},planning:{id:'care',task:'care::maintain',arguments:{actor:{kind:'actor',value:'ship'}},import:[{path:lib,as:'care',names:['maintain']}]}});
const pack=(source=world(),include=true)=>new WorkshopDocument(createStoreZip([{path:'scenarios.toml',text:'[pack]\nformat=1\nid="plan"\nname="Planning"\nversion="1"\n'}, {path,text:source},...(include?[{path:lib,text:library}]:[])]));
const deps=()=>({base_files:{[lib]:library}});
const settle=async()=>{for(let i=0;i<20;i++)await new Promise(resolve=>setTimeout(resolve,1));};
afterEach(()=>document.body.replaceChildren());
describe('accepted imported domain source',()=>{
  it('saves declaration and real library as one validated history transaction, preserving the rest of the World',async()=>{
    const draft=pack(),dependencies=deps(),before=draft.toFiles(),inspection=inspectPlanning(draft,dependencies,path);
    expect(planningWorlds(draft)).toEqual([path]);expect(activityWorlds(draft)).toEqual([path]);expect(inspection.libraries[0].library.task[0].method).toHaveLength(2);
    const runtime={validate:vi.fn(async(_,candidate)=>({accepted:parse(candidate.read(path)).planning.id==='changed'&&parse(candidate.read(lib)).task[0].method[0].id==='new-first'}))};
    await applyPlanning({draft,runtime,dependencies,path,rootSource:inspection.rootSource.replace('care"','changed"'),libraryPath:lib,librarySource:library.replace('spare-team','new-first')});
    expect(runtime.validate).toHaveBeenCalledOnce();expect(parse(draft.read(path)).script).toEqual(parse(world()).script);expect(parse(draft.read(path)).entity).toEqual(parse(world()).entity);
    const exported=new WorkshopDocument(draft.archive());expect(exported.read(lib)).toContain('new-first');
    draft.undo();expect(draft.toFiles()).toEqual(before);expect(draft.snapshot().history.redo).toHaveLength(1);draft.redo();expect(draft.read(lib)).toContain('new-first');
  });
  it('defers capability/import acceptance to backend and refuses an entire two-file candidate without writes',async()=>{
    const draft=pack(),before=draft.snapshot(),inspection=inspectPlanning(draft,deps(),path);
    await expect(applyPlanning({draft,runtime:{validate:async()=>({accepted:false,findings:[{message:'unsupported offered hull'}]})},dependencies:deps(),path,
      rootSource:inspection.rootSource.replace('care"','changed"'),libraryPath:lib,librarySource:library.replace('spare-team','unsupported')})).rejects.toThrow();expect(draft.snapshot()).toEqual(before);
  });
  it('requires explicit inherited library materialisation but allows independent root edits and empty disable',async()=>{
    const draft=pack(world(),false),dependencies=deps(),inspection=inspectPlanning(draft,dependencies,path),runtime={validate:async()=>({accepted:true})};
    await applyPlanning({draft,runtime,dependencies,path,rootSource:inspection.rootSource.replace('care"','changed"'),libraryPath:lib,librarySource:library});expect(draft.read(lib)).toBeUndefined();
    const before=draft.snapshot();await expect(applyPlanning({draft,runtime,dependencies,path,rootSource:inspectPlanning(draft,dependencies,path).rootSource,libraryPath:lib,librarySource:library+'\n# override\n'})).rejects.toThrow(/include/);expect(draft.snapshot()).toEqual(before);
    await applyPlanning({draft,runtime,dependencies,path,rootSource:inspectPlanning(draft,dependencies,path).rootSource,libraryPath:lib,librarySource:library+'\n# override\n',materialize:true});expect(draft.read(lib)).toContain('# override');
    await applyPlanning({draft,runtime,dependencies,path,rootSource:''});expect(parse(draft.read(path)).planning).toBeUndefined();expect(parse(draft.read(path)).script).toEqual(parse(world()).script);
  });
  it('refuses misleading source headers and unrelated library writes transactionally',async()=>{
    const source=world().replace(/^setup = .*$/m,"setup = '''\nfn unchanged(ctx) {}\n/*\n[planning]\nid='false-header'\n*/\n'''"),draft=pack(source),before=draft.snapshot();const runtime={validate:async()=>({accepted:true})};
    await expect(applyPlanning({draft,runtime,dependencies:deps(),path,rootSource:inspectPlanning(pack(),deps(),path).rootSource})).rejects.toThrow();expect(draft.snapshot()).toEqual(before);
    const actual=pack(),original=actual.snapshot();await expect(applyPlanning({draft:actual,runtime,dependencies:deps(),path,rootSource:inspectPlanning(actual,deps(),path).rootSource,libraryPath:'assets/worlds/foreign.toml',librarySource:library,materialize:true})).rejects.toThrow(/imported/);expect(actual.snapshot()).toEqual(original);
  });
});
describe('real planning authoring panel',()=>{
  it('preserves focused source and refreshes same-revision replaced packs, then invokes existing disposable Test',async()=>{
    let draft=pack();const operations=createWorkshopOperations(),changed=vi.fn(),test=vi.fn();const panel=mountWorkshopPlanning({root:document.body,runtime:{dependencies:async()=>deps(),validate:async()=>({accepted:true})},provider:{capabilities:{edit:true}},draft:()=>draft,...operations.panel(),changed,test});
    await settle();const area=panel.node.querySelector('#workshop-planning-source');area.focus();area.value+='\n# held edit';area.setSelectionRange(3,3);panel.refresh();expect(document.activeElement).toBe(area);expect(area.value).toContain('held edit');expect(area.selectionStart).toBe(3);
    draft=pack(world().replace('id = "care"','id = "replacement"'));panel.refresh();expect(panel.node.querySelector('#workshop-planning-declaration').value).toContain('replacement');panel.node.querySelector('#workshop-planning-test').click();expect(test).toHaveBeenCalledWith(path);panel.dispose();
  });
  it('rejects stale document identity after pending validation without editing either history or reporting success',async()=>{
    let draft=pack(),finish;const original=draft,operations=createWorkshopOperations(),changed=vi.fn();const panel=mountWorkshopPlanning({root:document.body,runtime:{dependencies:async()=>deps(),validate:()=>new Promise(resolve=>{finish=resolve;})},provider:{capabilities:{edit:true}},draft:()=>draft,...operations.panel(),changed});
    await settle();panel.node.querySelector('#workshop-planning-source').value+='\n# pending';panel.node.querySelector('#workshop-planning-save').click();await settle();draft=pack(world().replace('id = "care"','id = "replacement"'));const before=draft.snapshot();finish({accepted:true});await settle();expect(draft.snapshot()).toEqual(before);expect(original.read(lib)).toBe(library);expect(changed).not.toHaveBeenCalled();expect(operations.busy()).toBe(false);panel.dispose();
  });
});

describe('temporal declaration and primitive source authoring',()=>{
  it('keeps nested profiles and forecast phases in the same validated undoable exported transaction',async()=>{
    const draft=pack(),inspection=inspectPlanning(draft,deps(),path),before=draft.toFiles();
    const root=parse(inspection.rootSource);
    root.planning.temporal={profile:[{min_operators:0,deadline_secs:90},{min_operators:2,deadline_secs:50}]};
    const libSource=parse(library);
    const primitive=libSource.task.find(task=>task.kind==='primitive');
    primitive.forecast={min_secs:1,max_secs:5,recovery_secs:1,phase:[{kind:'active',offset_secs:0,duration_secs:1,attention_permille:1000}]};
    const runtime={validate:vi.fn(async(_,candidate)=>({accepted:parse(candidate.read(path)).planning.temporal.profile[1].deadline_secs===50&&parse(candidate.read(lib)).task.find(task=>task.kind==='primitive').forecast.max_secs===5}))};
    await applyPlanning({draft,runtime,dependencies:deps(),path,rootSource:stringify(root),libraryPath:lib,librarySource:stringify(libSource)});
    expect(runtime.validate).toHaveBeenCalledOnce();
    const exported=new WorkshopDocument(draft.archive());
    expect(parse(exported.read(path)).planning.temporal).toEqual(root.planning.temporal);
    expect(parse(exported.read(lib)).task.find(task=>task.kind==='primitive').forecast).toEqual(primitive.forecast);
    draft.undo();expect(draft.toFiles()).toEqual(before);draft.redo();expect(parse(draft.read(path)).planning.temporal.profile).toHaveLength(2);
  });
});


describe('existing promise root binding authoring',()=>{
  it('roundtrips optional required commitment through backend validation and ordinary accepted history',async()=>{
    const draft=pack(),inspection=inspectPlanning(draft,deps(),path),before=draft.toFiles(),root=parse(inspection.rootSource);
    root.planning.required_commitment='maintenance-assistance';
    const runtime={validate:vi.fn(async(_,candidate)=>({accepted:parse(candidate.read(path)).planning.required_commitment==='maintenance-assistance'}))};
    await applyPlanning({draft,runtime,dependencies:deps(),path,rootSource:stringify(root)});
    expect(runtime.validate).toHaveBeenCalledOnce();expect(inspectPlanning(new WorkshopDocument(draft.archive()),deps(),path).root.required_commitment).toBe('maintenance-assistance');
    expect(parse(draft.read(path)).script).toEqual(parse(world()).script);
    draft.undo();expect(draft.toFiles()).toEqual(before);draft.redo();expect(parse(draft.read(path)).planning.required_commitment).toBe('maintenance-assistance');
  });
});
