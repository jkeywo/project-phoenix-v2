import {expect,it,vi} from 'vitest';
import {parse,stringify} from 'smol-toml';
import {WorkshopDocument} from '../workshop-document.js';
import {createStoreZip} from '../mod-pack-export.js';
import {guidedProfile,validateGuidedProfile,prepareGuidedProfileOperation,applyGuidedProfileOperation} from '../workshop-guided-profile.js';
import {resolveTemplate} from '../entity-includes.js';
const path='assets/entities/ship.toml',base='assets/entities/base.toml',deps={base_files:{},packs:[]};
const source=stringify({name:'ship',torpedoes:{count:8,initial_count:3,speed:15,lifespan:20,durability:24,tubes:[{id:'fore',facing_deg:0,fire_arc_deg:90,impact:{damage_type:'physical',penetration:20}}],seeker:{range:50,fire_arc_deg:60,minimum_transmission:0.1},receiver:{range:200,minimum_transmission:0.1,maximum_sample_age_secs:2}},weapon:[{id:'guided-reference',strategy_id:'fore',mount:{facing_deg:0,fire_arc_deg:90,range:300},delivery:{kind:'guided',volley_max:1},impact:{damage_type:'physical',penetration:20},ai:{idle:true}}],appearance:{color:[1,2,3]}});
const draft=(text=source)=>new WorkshopDocument(createStoreZip([{path:'scenarios.toml',text:'[pack]\nformat=1\nid="mine"\nname="Mine"\nversion="1"\n'},{path,text}]));
it('edits one shared physical profile and matching generic range while preserving actual tubes/ammunition/unrelated source',()=>{
 const current=draft(),original=parse(source),profile=guidedProfile(original);profile.speed=20;profile.receiver.accept_external_illumination=true;
 const after=prepareGuidedProfileOperation(current,deps,{path,profile}).changes[0].after,value=parse(after);
 expect(value.torpedoes.tubes).toEqual(original.torpedoes.tubes);expect(value.torpedoes.count).toBe(8);expect(value.torpedoes.initial_count).toBe(3);
 expect(value.weapon[0].mount.range).toBe(400);expect(value.torpedoes.receiver.accept_external_illumination).toBe(true);expect(value.appearance).toEqual(original.appearance);
});
it('uses explicit false tombstones to disable inherited equipment without editing the supplying source',()=>{
 const current=draft('includes=["base.toml"]\n# own\n'),dependencies={base_files:{[base]:source},packs:[]},profile=guidedProfile(parse(source));profile.seeker=false;profile.receiver=false;
 expect(()=>prepareGuidedProfileOperation(current,dependencies,{path,profile})).toThrow(/materialise/);
 const after=prepareGuidedProfileOperation(current,dependencies,{path,profile,materialise:true}).changes[0].after;
 const result=resolveTemplate(path,{[path]:after,[base]:source},parse).resolved.value;
 expect(result.torpedoes.seeker).toBe(false);expect(result.torpedoes.receiver).toBe(false);expect(result.torpedoes.tubes).toEqual(parse(source).torpedoes.tubes);
 expect(current.read(path)).toContain('# own');expect(dependencies.base_files[base]).toBe(source);
});
it('rejects true/tiny/overflowing/unknown equipment shapes before accepted source changes',()=>{
 const initial=guidedProfile(parse(source));
 for(let fault=0;fault<6;fault++){const profile=structuredClone(initial);
 if(fault===0)profile.receiver=true;if(fault===1)profile.seeker=true;if(fault===2)profile.receiver.maximum_sample_age_secs=1e-100;
 if(fault===3)profile.speed=Infinity;if(fault===4)profile.seeker.secret_range=200;if(fault===5)profile.damage_hull=0.5;
 expect(()=>validateGuidedProfile(profile)).toThrow();}
});
it('uses actual document acceptance, undo/redo, reload and no-write rejection for shared profile changes',async()=>{
 const current=draft(),profile=guidedProfile(parse(source));profile.receiver.accept_external_illumination=true;
 const runtime={validate:vi.fn(async()=>({accepted:true,findings:[]}))};
 await applyGuidedProfileOperation({draft:current,runtime,dependencies:deps,operation:{path,profile}});const changed=current.read(path);
 expect(runtime.validate).toHaveBeenCalledTimes(1);expect(parse(changed).torpedoes.receiver.accept_external_illumination).toBe(true);
 current.undo();expect(current.read(path)).toBe(source);current.redo();expect(current.read(path)).toBe(changed);
 const reloaded=new WorkshopDocument(current.archive());expect(reloaded.read(path)).toBe(changed);
 runtime.validate.mockResolvedValueOnce({accepted:false,findings:[{message:'actual owner refused'}]});profile.speed=25;
 await expect(applyGuidedProfileOperation({draft:current,runtime,dependencies:deps,operation:{path,profile}})).rejects.toThrow();expect(current.read(path)).toBe(changed);
});
