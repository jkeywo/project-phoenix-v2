const object=value=>value!==null&&typeof value==='object'&&!Array.isArray(value);
const finite=value=>typeof value==='number'&&Number.isFinite(Math.fround(value));
const identity=value=>typeof value==='string'&&value.trim().length>0&&!/[\u0000-\u001f\u007f]/.test(value);
const stamp=value=>Number.isSafeInteger(value)&&value>=0;
const point=value=>Array.isArray(value)&&value.length===3&&value.every(finite);

/** Exact already-permitted typed drive publications. No aggregate/scene fallback. */
export function buildTransitDrives(entries,{systemIds=[],controlSources={}}={}) {
  const result=[],seen=new Set();
  for(const {systemId,data} of entries || []) {
    if(!object(data)||!identity(systemId)||data.owner!==systemId||seen.has(systemId)
      ||!['idle','preparing','committed','arrived'].includes(data.phase)
      ||!finite(data.preparation_fraction)||data.preparation_fraction<0||data.preparation_fraction>1
      ||!Number.isSafeInteger(data.fuel_required)||data.fuel_required<=0
      ||(data.last_step_tick!=null&&!stamp(data.last_step_tick))
      ||(data.waiting!=null&&!['unavailable','unfunded','out_of_range','no_placement'].includes(data.waiting))
      ||(data.request!=null&&(!object(data.request)||!identity(data.request.objective_id)||!stamp(data.request.revision)))
      ||(data.course!=null&&(!object(data.course)||data.course.kind!=='local'||!point(data.course.point)||!stamp(data.course.course_generation)))
      ||(data.actual_arrival!=null&&!point(data.actual_arrival))
      ||(data.phase==='arrived')!==(data.actual_arrival!=null)
      ||(data.phase!=='idle'&&data.course==null)
      ||(data.phase==='idle'&&(data.preparation_fraction!==0||data.request!=null))
      ||(['committed','arrived'].includes(data.phase)&&data.preparation_fraction!==1)) continue;
    seen.add(systemId);
    const controllable=systemIds.includes(systemId)&&controlSources[systemId]==='Human';
    result.push({system_id:systemId,phase:data.phase,preparation_fraction:data.preparation_fraction,
      fuel_required:data.fuel_required,controllable,
      can_begin:controllable&&data.can_begin===true&&['idle','arrived'].includes(data.phase),
      can_cancel:controllable&&data.can_cancel===true&&data.phase==='preparing',
      waiting:data.waiting??null,request:data.request?structuredClone(data.request):null,
      course:data.course?structuredClone(data.course):null,
      actual_arrival:data.actual_arrival?[...data.actual_arrival]:null,last_step_tick:data.last_step_tick??null});
  }
  return result.sort((a,b)=>a.system_id.localeCompare(b.system_id));
}
