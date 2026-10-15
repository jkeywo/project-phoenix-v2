import {t} from './strings.js';
import {activityWorlds,activityChoices,inspectActivities,applyActivities} from '../editor/workshop-activities.js';
import {runWorkshopMutation} from './workshop-edit-session.js';

/** Base-world contracts only. This edits source; it never dispatches work. */
export function mountWorkshopActivities({root,provider,runtime,draft:getDraft,busy,setBusy,changed=()=>{},test=()=>false,attach=true}) {
  const doc=root.ownerDocument,prefix='workshop-activities-';
  const el=(tag,id,key)=>{const node=doc.createElement(tag);if(id)node.id=prefix+id;if(key)node.textContent=t('workshop.activities.'+key);return node;};
  const input=(id,value='',type='text')=>{const node=el('input',id);node.type=type;node.value=value;return node;};
  const button=(id,key,action)=>{const node=el('button',id,key);node.type='button';node.onclick=action;return node;};
  const label=(node,key)=>{const wrap=el('label',null,key);wrap.htmlFor=node.id;wrap.append(node);return wrap;};
  const select=(id,values,value)=>{const node=el('select',id);for(const [key,text]of values){const option=el('option');option.value=key;option.textContent=text;node.append(option);}node.value=value ?? node.options[0]?.value ?? '';return node;};
  const node=el('section','panel'),world=el('select','world'),definitions=el('div','definitions'),instances=el('div','instances'),status=el('p','status');status.setAttribute('role','status');
  let dependencies=null,inspection=null,disposed=false,loading=false,generation=0,serial=0,observedDraft=null,observed=null,definitionRows=[],instanceRows=[];
  const json=value=>JSON.stringify(value);
  const allowed=()=>!disposed&&!loading&&!busy()&&!!inspection;
  function controls(){for(const item of node.querySelectorAll('input,select,button'))item.disabled=!allowed();world.disabled=disposed||loading||busy()||!world.options.length;}
  function addDefinition(value={id:'',state:[]}) {
    const fieldset=el('fieldset'),name=input('definition-'+serial++,value.id),states=el('div');
    const row={fieldset,name,states:[]};definitionRows.push(row);
    fieldset.append(label(name,'definition'),states);
    function addState(value={id:'',when:{kind:'accepted'}}) {
      const box=el('fieldset'),name=input('state-'+serial++,value.id),when=select('condition-'+serial++,['accepted','achieved','settled','received','returned','communication_read','communication_reported','communication_declined','scenario_applied'].map(key=>[key,t('workshop.activities.condition.'+key)]),value.when.kind);
      const outcome=select('outcome-'+serial++,['completed','cancelled','failed'].map(key=>[key,t('workshop.activities.outcome.'+key)]),value.when.outcome);
      const previous=input('application-state-'+serial++,value.when.state || '');
      const script=input('script-'+serial++,value.call?.script || ''),fn=input('function-'+serial++,value.call?.function || '');
      const state={box,name,when,outcome,previous,script,fn};row.states.push(state);
      box.append(label(name,'state'),label(when,'condition'),label(outcome,'outcome'),label(previous,'application_state'),label(script,'script'),label(fn,'function'),
        button(null,'remove',()=>{row.states.splice(row.states.indexOf(state),1);box.remove();}));
      const refresh=()=>{outcome.hidden=when.value!=='settled';previous.hidden=when.value!=='scenario_applied';};when.addEventListener('change',refresh);refresh();states.append(box);controls();
    }
    const seen=new Set();let cursor=value.initial;
    while(cursor!=null&&!seen.has(cursor)){const state=value.state.find(row=>row.id===cursor);if(!state)break;seen.add(cursor);addState(state);cursor=state.next;}
    for(const state of value.state)if(!seen.has(state.id))addState(state);
    fieldset.append(button(null,'add_state',()=>addState()),button(null,'remove',()=>{definitionRows.splice(definitionRows.indexOf(row),1);fieldset.remove();}));
    definitions.append(fieldset);controls();
  }
  function addInstance(value={id:'',definition:'',request:{kind:'repair'}}) {
    const fieldset=el('fieldset'),name=input('instance-'+serial++,value.id),definition=input('binding-'+serial++,value.definition);
    const actor=select('actor-'+serial++,inspection.actors.map(row=>[row.id,row.id]),value.request.actor),kind=select('kind-'+serial++,[['repair',t('workshop.activities.repair')],['delivery',t('workshop.activities.delivery')],['communication',t('workshop.activities.communication')]],value.request.kind),body=el('div');
    const row={fieldset,name,definition,actor,kind,read:()=>null};instanceRows.push(row);
    function detail(request={}) {
      body.replaceChildren();const profile=inspection.actors.find(row=>row.id===actor.value)?.profiles[0],choices=activityChoices(profile || {});
      if(kind.value==='repair') {
        const owners=choices.systems.filter(row=>row.kind==='repair'),owner=select('owner-'+serial++,owners.map(row=>[row.id,row.id]),request.owner);
        const team=input('team-'+serial++,request.team ?? 0,'number');team.min='0';team.step='1';team.max=String((profile?.repair?.repair_team_count || 1)-1);
        const target=select('target-'+serial++,choices.targets.map(row=>[json(row.target),row.target.kind==='compartment'?row.target.compartment:row.target.system+'/'+row.target.component]),request.target&&json(request.target));
        const goal=select('goal-'+serial++,['structure','patch','full'].map(key=>[key,t('workshop.activities.goal.'+key)]),request.goal);
        const visits=[['exact',t('workshop.activities.exact')],...choices.systems.map(row=>[json({type:'System',data:row.id}),row.id]),
          ...(profile?.console || []).map(row=>[json({type:'Console',data:row.id}),'Console: '+row.id]),...(profile?.station || []).map(row=>[json({type:'Station',data:row.id}),'Station: '+row.id]),[json({type:'Core'}),'Core']];
        const visit=select('visit-'+serial++,visits,request.request?.type==='Work'?'exact':request.request&&json(request.request));
        body.append(label(owner,'owner'),label(team,'team'),label(visit,'visit'),label(target,'target'),label(goal,'goal'));
        row.read=()=>{const work=JSON.parse(target.value);return{kind:'repair',actor:actor.value,owner:owner.value,team:team.valueAsNumber,
          request:visit.value==='exact'?{type:'Work',data:work}:JSON.parse(visit.value),target:work,goal:goal.value};};
      } else if(kind.value==='communication') {
        const owner=select('owner-'+serial++,[...new Set(choices.communications.map(row=>row.owner))].map(row=>[row,row]),request.owner);
        const channel=select('channel-'+serial++,choices.communications.map(row=>[row.channel,row.channel]),request.channel);
        const recipients=inspection.actors.filter(row=>row.id!==actor.value && !row.id.startsWith('slot:') && row.profiles.length && row.profiles.every(profile=>profile.comms?.radio));
        const recipient=select('recipient-'+serial++,recipients.map(row=>[row.id,row.id]),request.recipient);
        body.append(label(owner,'owner'),label(recipient,'recipient'),label(channel,'channel'));
        row.read=()=>({kind:'communication',actor:actor.value,owner:owner.value,recipient:recipient.value,channel:channel.value,request:'report_status'});
      } else {
        const route=select('route-'+serial++,choices.deliveries.map(row=>[json(row),row.owner+': '+row.source+' → '+row.destination+' ('+row.commodity+')']),undefined);
        const found=choices.deliveries.find(row=>row.owner===request.owner&&row.source===request.source&&row.destination===request.destination&&row.commodity===request.commodity);if(found)route.value=json(found);
        const amount=input('amount-'+serial++,request.amount ?? 1,'number');amount.min='1';amount.step='1';
        body.append(label(route,'route'),label(amount,'amount'));
        row.read=()=>{const {max,...path}=JSON.parse(route.value);return{kind:'delivery',actor:actor.value,...path,amount:amount.valueAsNumber};};
      }
      controls();
    }
    actor.addEventListener('change',()=>detail());kind.addEventListener('change',()=>detail());
    fieldset.append(label(name,'instance'),label(definition,'definition'),label(actor,'actor'),label(kind,'kind'),body,
      button(null,'remove',()=>{instanceRows.splice(instanceRows.indexOf(row),1);fieldset.remove();}));instances.append(fieldset);detail(value.request);controls();
  }
  function inspect(force=false) {
    const draft=getDraft(),paths=activityWorlds(draft).sort(),selected=paths.includes(world.value)?world.value:paths[0] || '';
    if([...world.options].map(row=>row.value).join('\n')!==paths.join('\n'))world.replaceChildren(...paths.map(path=>{const option=el('option');option.value=option.textContent=path;return option;}));world.value=selected;
    if(!draft||!dependencies||!selected){inspection=null;controls();return;}
    const key=json([draft.sourceRevision,selected,dependencies]);if(!force&&observedDraft===draft&&observed===key){controls();return;}
    try{inspection=inspectActivities(draft,dependencies,selected);observedDraft=draft;observed=key;definitions.replaceChildren();instances.replaceChildren();definitionRows=[];instanceRows=[];
      inspection.definitions.forEach(addDefinition);inspection.situations.forEach(addInstance);status.textContent='';}
    catch(error){inspection=null;status.textContent=t('workshop.activities.refused',{reason:error.message});}controls();
  }
  async function load(){if(disposed||loading||dependencies)return;loading=true;controls();const token=++generation;
    try{const value=await runtime.dependencies();if(!disposed&&token===generation)dependencies=value;}
    catch(error){status.textContent=t('workshop.activities.refused',{reason:error.message});}
    finally{if(!disposed&&token===generation){loading=false;inspect(true);}}}
  const save=button('save','save',()=>{
    if(!allowed())return;const draft=getDraft(),path=world.value,token=++generation;
    const current=()=>!disposed&&token===generation&&getDraft()===draft&&world.value===path;
    void runWorkshopMutation({setBusy,current,start:controls,invoke:()=>{
      const values=definitionRows.map(row=>({id:row.name.value,initial:row.states[0]?.name.value || '',state:row.states.map((state,index)=>({id:state.name.value,
        when:{kind:state.when.value,...(state.when.value==='settled'?{outcome:state.outcome.value}:state.when.value==='scenario_applied'?{state:state.previous.value}:{})},
        ...(state.script.value||state.fn.value?{call:{script:state.script.value,function:state.fn.value}}:{}),
        ...(index+1<row.states.length?{next:row.states[index+1].name.value}:{})}))}));
      return applyActivities({draft,provider,runtime,dependencies,path,definitions:values,situations:instanceRows.map(row=>({id:row.name.value,definition:row.definition.value,request:row.read()})),current});},
      success:()=>{changed(path);inspect(true);status.textContent=t('workshop.changed');},error:error=>{status.textContent=t('workshop.activities.refused',{reason:error.report?.findings?.[0]?.message || error.message});},release:controls});
  });
  const runTest=button('test',null,()=>{if(allowed()&&test(world.value)!==true)status.textContent=t('workshop.activities.test_unavailable');});runTest.textContent=t('workshop.test_heading');
  node.append(el('h3',null,'title'),el('p',null,'scope'),label(world,'world'),definitions,button('add-definition','add_definition',()=>addDefinition()),instances,
    button('add-instance','add_instance',()=>addInstance()),save,runTest,status);
  world.addEventListener('change',()=>{generation++;inspect(true);});if(attach)root.append(node);controls();void load();
  return{node,refresh({hidden=false}={}){node.hidden=hidden;if(!hidden){void load();inspect();}},dispose(){disposed=true;generation++;node.remove();}};
}
