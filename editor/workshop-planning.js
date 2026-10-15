import {parse, stringify} from 'smol-toml';
import {acceptWorkshopChanges} from './workshop-acceptance.js';

const object=value=>value!==null && typeof value==='object' && !Array.isArray(value);
const stable=value=>JSON.stringify(value,(_,row)=>object(row)?Object.fromEntries(Object.keys(row).sort().map(key=>[key,row[key]])):row);
const pathOk=path=>typeof path==='string' && path.startsWith('assets/') && path.endsWith('.toml')
  && !/[\\:\0]/.test(path) && path.split('/').every(row=>row && row!=='.' && row!=='..');
function sources(draft,dependencies) {
  return {...(dependencies?.base_files || {}),...Object.fromEntries(draft.paths().filter(path=>!draft.isBinary(path)).map(path=>[path,draft.read(path)]))};
}
export function planningWorlds(draft) {
  return draft.paths().filter(path=>path.startsWith('assets/worlds/') && path.endsWith('.toml') && !draft.isBinary(path))
    .filter(path=>{try{return !object(parse(draft.read(path)).domain);}catch{return true;}}).sort();
}
function spans(source) {
  const headers=[...source.matchAll(/^[ \t]*\[{1,2}([^\r\n]+?)\]{1,2}[ \t]*(?:#.*)?(?:\r?\n|$)/gm)];
  return headers.map((row,index)=>({key:row[1].replace(/\]$/,'').trim(),start:row.index,end:headers[index+1]?.index ?? source.length}))
    .filter(row=>row.key==='planning' || row.key.startsWith('planning.'));
}
export function planningRootSource(source) {return spans(source).map(row=>source.slice(row.start,row.end)).join('');}
function rootEdit(before,fragment) {
  const parsed=parse(before),value=fragment.trim()?parse(fragment):{};
  if(Object.keys(value).some(key=>key!=='planning') || (value.planning!==undefined && !object(value.planning))) throw Error('Edit only the World planning declaration in this field.');
  if(fragment===planningRootSource(before))return before;
  let after=before;
  for(const row of spans(before).reverse())after=after.slice(0,row.start)+after.slice(row.end);
  if(fragment.trim()) {const nl=before.includes('\r\n')?'\r\n':'\n';after+=(after.endsWith('\n')?nl:nl+nl)+fragment.replace(/\r?\n/g,nl);}
  const result=parse(after),rest=row=>{const copy=structuredClone(row);delete copy.planning;return copy;};
  if(stable(result.planning)!==stable(value.planning)||stable(rest(parsed))!==stable(rest(result))) throw Error('Planning edit must preserve every unrelated World field; expand inline or misleading source tables first.');
  return after;
}
function imports(root,files) {
  const rows=[],seen=new Set();
  function visit(imports,depth) {
    if(depth>8)throw Error('Imported planning source exceeds the supported depth.');
    for(const row of imports || []) {
      if(!pathOk(row.path))throw Error('Select an actual relative asset TOML library.');
      if(seen.has(row.path))continue;
      if(seen.size>=32)throw Error('Imported planning source exceeds the supported library bound.');
      seen.add(row.path);
      const text=files[row.path],library=typeof text==='string'?parse(text):null;
      rows.push({path:row.path,text,namespace:row.as,names:row.names || [],library});
      if(library)visit(library.import,depth+1);
    }
  }
  visit(root?.import,0);return rows;
}
export function inspectPlanning(draft,dependencies,path) {
  const source=draft.read(path);if(typeof source!=='string')throw Error('Select an editable World source.');
  const world=parse(source),files=sources(draft,dependencies);
  return {path,source,root:world.planning,rootSource:planningRootSource(source),libraries:imports(world.planning,files)};
}
/** One accepted transaction; the backend owns import closure, typed parameters,
 * alternative capability support and Scenario validation. No form-only planner. */
export async function applyPlanning({draft,provider,runtime,dependencies,path,rootSource,libraryPath='',librarySource='',materialize=false,current=()=>true}) {
  return acceptWorkshopChanges({draft,provider,runtime,dependencies,current,prepare:(captured,effective)=>{
    const before=captured.read(path),after=rootEdit(before,rootSource),changes=before===after?[]:[{path,before,after}];
    if(libraryPath) {
      const files=sources(captured,effective);files[path]=after;files[libraryPath]=librarySource;
      if(!imports(parse(after).planning,files).some(row=>row.path===libraryPath))throw Error('Edit a library imported by this World.');
      if(!object(parse(librarySource).domain))throw Error('Keep the library domain header in its own source.');
      const original=captured.read(libraryPath);
      if(original===undefined&&!materialize){
        if(librarySource.replace(/\r\n/g,'\n')===String(effective?.base_files?.[libraryPath] || '').replace(/\r\n/g,'\n'))return changes;
        throw Error('Explicitly include the inherited library override before editing it.');
      }
      if(original!==librarySource)changes.push({path:libraryPath,before:original ?? null,after:librarySource});
    }
    return changes;
  }});
}
export function rootTemplate() {return stringify({planning:{id:'maintenance',task:'maintenance::maintain',arguments:{actor:{kind:'actor',value:''}},import:[{path:'assets/worlds/domains/maintenance.toml',as:'maintenance',names:['maintain']}]}});}
