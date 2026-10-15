import '../strings-boot.js';
import {t} from '../strings.js';
import {PhElement,phDefine} from './ph-element.js';

export class PhTransitDrives extends PhElement {
  template(){return `<style>:host{display:block;font:var(--text-sm) monospace}:host([hidden]){display:none}section{margin:6px 0}p{margin:3px 0;overflow-wrap:anywhere}button{font:inherit;margin-right:5px}</style><h4>${t('helm.transit.title')}</h4><p>${t('helm.transit.hint')}</p><div id="drives"></div>`;}
  onTemplate(){
    this.rows=new Map();
    for(const kind of ['keydown','keyup'])this.shadowRoot.addEventListener(kind,event=>event.stopPropagation());
  }
  render(value){
    const drives=Array.isArray(value)?value:[];this.hidden=!drives.length;
    const present=new Set(drives.map(row=>row.system_id));
    for(const [id,row]of this.rows)if(!present.has(id)){row.node.remove();this.rows.delete(id);}
    for(const drive of drives){
      let row=this.rows.get(drive.system_id);
      if(!row){
        const doc=this.ownerDocument,node=doc.createElement('section'),heading=doc.createElement('strong'),status=doc.createElement('p'),course=doc.createElement('p'),result=doc.createElement('p');
        node.dataset.systemId=drive.system_id;
        const buttons=['begin','cancel'].map(kind=>{const button=doc.createElement('button');button.type='button';button.textContent=t(`helm.transit.${kind}`);button.dataset.kind=kind;
          button.addEventListener('click',()=>{const current=this.state?.find(row=>row.system_id===node.dataset.systemId);
            if(current?.controllable&&current[`can_${kind}`])window.activateSemanticAction?.(`helm.transit-${kind}`,{context:'helm',source:'control',detail:{systemId:current.system_id}});});return button;});
        node.append(heading,status,course,result,...buttons);this.$('drives').append(node);row={node,heading,status,course,result,buttons};this.rows.set(drive.system_id,row);
      }
      row.heading.textContent=drive.system_id;
      row.status.textContent=t(`helm.transit.phase.${drive.phase}`)+` · ${Math.round(drive.preparation_fraction*100)}% · `+t('helm.transit.fuel',{amount:drive.fuel_required})
        +(drive.waiting?` · ${t(`helm.transit.wait.${drive.waiting}`)}`:'')+(!drive.controllable?` · ${t('helm.movement.delegated')}`:'');
      row.course.textContent=drive.course?t('helm.transit.course',{point:drive.course.point.map(value=>value.toFixed(1)).join(', '),generation:drive.course.course_generation}):t('helm.transit.no_course');
      row.result.textContent=drive.actual_arrival?t('helm.transit.actual',{point:drive.actual_arrival.map(value=>value.toFixed(1)).join(', '),tick:drive.last_step_tick??'?'}):'';
      row.buttons[0].disabled=!drive.can_begin;row.buttons[1].disabled=!drive.can_cancel;
    }
  }
}
phDefine('ph-transit-drives',PhTransitDrives);
