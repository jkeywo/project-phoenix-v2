import {it,expect,vi} from 'vitest';
import {parse,stringify} from 'smol-toml';
import {WorkshopDocument} from '../workshop-document.js';
import {createStoreZip} from '../mod-pack-export.js';
import {validateLoaderGroups,guardLoaderReferences} from '../workshop-loader-model.js';
import {applySharedLoaders} from '../workshop-shared-loaders.js';
const path='assets/entities/own.toml',base='assets/entities/base.toml';
const entity={name:'own',torpedoes:{count:6,tubes:[{id:'port',facing_deg:0,fire_arc_deg:90},{id:'starboard',facing_deg:0,fire_arc_deg:90}]}};
const groups=[{id:'forward',tubes:['port','starboard'],work_rate:1}];
it('validates actual distinct guided membership and finite group capacity',()=>{
 expect(validateLoaderGroups(entity,groups)).toEqual(groups);
 for(const bad of [[{...groups[0],tubes:['port','port']}],[{...groups[0],tubes:['port','missing']}],[{...groups[0],work_rate:3}],[{...groups[0],work_rate:Infinity}]])expect(()=>validateLoaderGroups(entity,bad)).toThrow();
 expect(()=>guardLoaderReferences(entity,{...entity,torpedoes:{...entity.torpedoes,tubes:[entity.torpedoes.tubes[0]],loader_groups:groups}})).toThrow();
});
it('materialises only native loader-group leaf through accepted history and rejects stale/native refusal',async()=>{
 const source=`includes=["base.toml"]
# untouched
`;
 const draft=new WorkshopDocument(createStoreZip([{path:'scenarios.toml',text:`[pack]
format=1
id="own"
name="Own"
version="1"
`},{path,text:source}]));
 const dependencies={base_files:{[base]:stringify(entity)},packs:[]};
 // Structural model of the native packet. Exact source preservation has its
 // separate real native document-owner proof.
 const runtime={validate:vi.fn(async()=>({accepted:true,findings:[]})),compose:vi.fn(async(files,request)=>{
   const local=parse(request.expected_source);for(const edit of request.edits){if(edit.path.length===1)local.torpedoes={};else local.torpedoes.loader_groups=parse('value='+edit.value_source).value;}return stringify(local);
 })};
 await expect(applySharedLoaders({draft,runtime,dependencies,operation:{path,groups}})).rejects.toThrow(/materialise/);
 await applySharedLoaders({draft,runtime,dependencies,operation:{path,groups,materialise:true}});const changed=draft.read(path);expect(parse(changed).torpedoes).toEqual({loader_groups:groups});
 draft.undo();expect(draft.read(path)).toBe(source);draft.redo();expect(draft.read(path)).toBe(changed);
 runtime.validate.mockResolvedValueOnce({accepted:false,findings:[{message:'actual owner refused'}]});await expect(applySharedLoaders({draft,runtime,dependencies,operation:{path,groups:[],materialise:true}})).rejects.toThrow();expect(draft.read(path)).toBe(changed);
});
