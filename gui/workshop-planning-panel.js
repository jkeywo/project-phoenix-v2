import {planningWorlds,inspectPlanning,applyPlanning,rootTemplate} from '../editor/workshop-planning.js';
import {runWorkshopMutation} from './workshop-edit-session.js';
import {t} from './strings.js';

/** Source authoring of the actual backend planning schema through the existing
 * chronological accepted-document operation and disposable World Test route. */
export function mountWorkshopPlanning({root,provider,runtime,draft:getDraft,busy,setBusy,changed=()=>{},test=()=>false,attach=true}) {
  const doc=root.ownerDocument,node=doc.createElement('section');node.className='workshop-planning';
  const el=(tag,id,key)=>{const element=doc.createElement(tag);if(id)element.id='workshop-planning-'+id;if(key)element.textContent=t('workshop.planning.'+key);return element;};
  const label=(input,key)=>{const row=el('label',null,key);row.htmlFor=input.id;row.append(input);return row;};
  const world=el('select','world'),declaration=el('textarea','declaration'),library=el('select','library'),source=el('textarea','source'),materialize=el('input','materialize'),summary=el('div','summary'),status=el('div','status');
  declaration.rows=10;source.rows=16;materialize.type='checkbox';
  let dependencies=null,inspection=null,observedDraft=null,revision=-1,loading=false,disposed=false,generation=0;
  const allowed=()=>!disposed&&!busy()&&inspection&&provider?.capabilities?.edit!==false;
  const controls=()=>{const active=!!allowed();world.disabled=disposed||busy()||provider?.capabilities?.edit===false||!world.options.length;for(const input of [declaration,library,save,add,runTest])input.disabled=!active;source.disabled=!active||!library.value;materialize.disabled=!active||!library.value||getDraft()?.read(library.value)!==undefined;};
  function showLibrary(){const row=inspection?.libraries.find(row=>row.path===library.value);source.value=row?.text || '';materialize.checked=false;summary.replaceChildren();
    if(row?.library){for(const task of row.library.task || []){const line=el('div');line.textContent=task.id+': '+(task.parameters || []).map(row=>row.name+' ('+row.kind+')').join(', ');summary.append(line);
      for(const method of task.method || []) {const detail=el('div');detail.textContent=method.id+': '+(method.step || []).map(step=>step.id+' = '+step.task).join('; ');summary.append(detail);
        for(const edge of method.edge || []){const dependency=el('div');dependency.textContent=edge.from+' -> '+edge.to+' ('+edge.when?.kind+')';summary.append(dependency);}}}}
    controls();}
  function inspect(force=false){const draft=getDraft(),paths=draft?planningWorlds(draft):[],selected=paths.includes(world.value)?world.value:paths[0] || '';
    if([...world.options].map(row=>row.value).join('\n')!==paths.join('\n'))world.replaceChildren(...paths.map(path=>{const option=el('option');option.value=option.textContent=path;return option;}));world.value=selected;
    if(!draft||!dependencies||!selected){inspection=null;controls();return;}
    if(!force&&observedDraft===draft&&revision===draft.sourceRevision&&inspection?.path===selected){controls();return;}
    try{inspection=inspectPlanning(draft,dependencies,selected);observedDraft=draft;revision=draft.sourceRevision;declaration.value=inspection.rootSource;
      const selectedLibrary=library.value;library.replaceChildren(el('option'),...inspection.libraries.map(row=>{const option=el('option');option.value=option.textContent=row.path;return option;}));
      library.value=inspection.libraries.some(row=>row.path===selectedLibrary)?selectedLibrary:inspection.libraries[0]?.path || '';showLibrary();status.textContent='';}
    catch(error){inspection=null;status.textContent=error.message;}controls();}
  async function load(){if(disposed||loading||dependencies)return;loading=true;const token=++generation;controls();try{const value=await runtime.dependencies();if(!disposed&&token===generation)dependencies=value;}
    catch(error){status.textContent=error.message;}finally{if(!disposed&&token===generation){loading=false;inspect(true);}}}
  const add=el('button','add','add');add.type='button';add.addEventListener('click',()=>{if(allowed())declaration.value=rootTemplate();});
  const save=el('button','save','save');save.type='button';save.addEventListener('click',()=>{if(!allowed())return;const draft=getDraft(),path=world.value,token=++generation;
    const value={rootSource:declaration.value,libraryPath:library.value,librarySource:source.value,materialize:materialize.checked};
    const current=()=>!disposed&&token===generation&&getDraft()===draft&&world.value===path;
    void runWorkshopMutation({setBusy,current,start:controls,invoke:()=>applyPlanning({draft,provider,runtime,dependencies,path,...value,current}),
      success:()=>{changed(path);inspect(true);status.textContent=t('workshop.changed');},error:error=>{status.textContent=error.report?.findings?.[0]?.message || error.message;},release:controls});});
  const runTest=el('button','test');runTest.type='button';runTest.textContent=t('workshop.test_heading');runTest.addEventListener('click',()=>{if(allowed())test(world.value);});
  world.addEventListener('change',()=>{generation++;inspect(true);});library.addEventListener('change',()=>{generation++;showLibrary();});
  node.append(el('h3',null,'title'),el('p',null,'scope'),label(world,'world'),label(declaration,'declaration'),add,label(library,'library'),label(materialize,'materialize'),label(source,'source'),summary,save,runTest,status);
  if(attach)root.append(node);controls();void load();
  return{node,refresh({hidden=false}={}){node.hidden=hidden;if(!hidden){void load();inspect();}},dispose(){disposed=true;generation++;node.remove();}};
}
