import { mountFormationCourseControls } from './formation-course-controls.js';
import { t, wireText } from '../strings.js';
import { mountRadarSharingControls } from './radar-sharing-controls.js';
import { NAVIGATION_SHARED_WAYPOINT_ACTION_ID, NAVIGATION_WORLD_ROUTE_ACTION_ID } from '../stations/navigation-actions.js';

export const navigationSharingState = view => ({ radar_sharing: view?.radar_sharing || [],
  received_chart: view?.received_chart || [], navigation_auto: !!view?.navigation_auto,
  formation: view?.formation || null,
  transit_routes: view?.transit_routes || [], navigation_system_id:view?.navigation_system_id,
  navigation_controllable:view?.navigation_controllable===true });

/** Fixed received observations: changing course always remains an explicit act. */
export function mountNavigationSharingControls(doc) {
  const node=doc.createElement('section'),sharing=mountRadarSharingControls(doc),list=doc.createElement('div'),rows=new Map();
  const routes=doc.createElement('select'),choose=doc.createElement('button'),routeBox=doc.createElement('div');
  choose.type='button';choose.textContent=t('navigation.transit.select');routes.setAttribute('aria-label',t('navigation.transit.known'));routeBox.append(routes,choose);
  let routeState=null,routeKey=null;choose.dataset.worldRoute='';routes.dataset.worldRoutes='';choose.addEventListener('click',()=>{if(choose.disabled||!routeState)return;doc.defaultView?.activateSemanticAction?.(NAVIGATION_WORLD_ROUTE_ACTION_ID,{source:'control',context:'navigation',detail:{id:routes.value}});});
  const formation=mountFormationCourseControls(doc);
  const title=doc.createElement('h4');title.textContent=t('navigation.sharing.received');
  node.append(sharing.node,title,list,routeBox,formation.node);node.dataset.navigationSharing='';
  node.addEventListener('keydown',event=>{if(!['Tab','Escape'].includes(event.key))event.stopPropagation();});
  return {node,update(state={}) {
    state ||= {};
    routeState=state;formation.update(state);
    const choices=Array.isArray(state.transit_routes)?state.transit_routes:[],selected=routes.value;
    const key=JSON.stringify(choices);if(key!==routeKey){routeKey=key;routes.replaceChildren(...choices.map(route=>{const option=doc.createElement('option');option.value=route.id;option.textContent=wireText(route.label);return option;}));if(choices.some(route=>route.id===selected))routes.value=selected;}
    choose.disabled=state.navigation_controllable!==true||!!state.navigation_auto||choices.length===0;routes.disabled=choose.disabled;routeBox.hidden=choices.length===0;
    const owners=(state.radar_sharing || []).filter(row=>row.role==='navigation');sharing.update(owners);
    const owner=owners.find(row=>row.controllable && row.can_operate),present=new Set();
    for(const sample of state.received_chart || []) {
      if(!sample?.provenance?.sample_id || !Array.isArray(sample.position) || sample.position.length!==2 || !sample.position.every(Number.isFinite)
        || !['waypoint','landmark'].includes(sample.kind?.kind))continue;
      const id=sample.provenance.sample_id;present.add(id);let row=rows.get(id);
      if(!row) {
        const group=doc.createElement('div'),text=doc.createElement('p'),button=doc.createElement('button');button.type='button';button.textContent=t('navigation.sharing.apply');
        row={group,text,button,value:null};button.addEventListener('click',()=>{
          if(!row.value || button.disabled)return;
          doc.defaultView?.activateSemanticAction?.(NAVIGATION_SHARED_WAYPOINT_ACTION_ID,{source:'control',context:'navigation',detail:{sample_id:row.value.sample_id,owner:row.value.owner}});
        });group.append(text,button);list.append(group);rows.set(id,row);
      }
      row.value={sample_id:id,owner:owner?.owner};row.button.disabled=!owner || !!state.navigation_auto;
      const name=sample.kind.kind==='landmark'?wireText(sample.kind.name):t('navigation.sharing.waypoint');
      row.text.textContent=t('navigation.sharing.sample',{name,x:sample.position[0],z:sample.position[1],source:wireText(sample.provenance.origin?.source),tick:sample.provenance.observed_tick});
    }
    for(const [id,row]of rows)if(!present.has(id)){row.value=null;row.group.remove();rows.delete(id);}
    title.hidden=rows.size===0;node.hidden=owners.length===0 && rows.size===0 && choices.length===0 && !state.formation;
  },dispose(){routeState=null;formation.dispose();sharing.dispose();for(const row of rows.values())row.value=null;rows.clear();node.remove();}};
}
