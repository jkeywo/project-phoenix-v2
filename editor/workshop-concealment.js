import {validateChannelInitials} from './workshop-protection.js';
export const initialConcealment=()=>({
  components:[{kind:'power_input',id:'input'},{kind:'concealment_emitter',id:'field',power_input:'input',enabled_channel:'enabled',config:{max_suppression:0.85,emission_scale:50}}],
  channels:[{name:'enabled',kind:'boolean',initial:{kind:'boolean',value:false}}],
});
export function validateConcealmentComposition(system,components,channels,entity=null) {
  if(system.kind!=='concealment')return;
  validateChannelInitials(channels);
  const emitter=components.find(row=>row.kind==='concealment_emitter'),config=emitter?.config;
  const id=value=>typeof value==='string'&&/^[a-z0-9-]+$/.test(value);
  const number=value=>typeof value==='number'&&Number.isFinite(Math.fround(value));
  if(!system.power_group||components.length!==2||!emitter||!components.every(row=>id(row.id))
    ||new Set(components.map(row=>row.id)).size!==2
    ||components.filter(row=>row.kind==='power_input'&&row.id===emitter.power_input).length!==1
    ||Object.keys(emitter).some(key=>!['kind','id','power_input','enabled_channel','config'].includes(key))
    ||components.some(row=>row.kind==='power_input'&&Object.keys(row).some(key=>!['kind','id'].includes(key)))
    ||channels.length!==1||!id(channels[0]?.name)||channels[0].name!==emitter.enabled_channel||channels[0].kind!=='boolean'
    ||!config||Object.keys(config).some(key=>!['max_suppression','emission_scale'].includes(key))
    ||!number(config.max_suppression)||Math.fround(config.max_suppression)<=0||Math.fround(config.max_suppression)>=1
    ||!number(config.emission_scale)||Math.fround(config.emission_scale)<=0)
    throw Error('Use one concealment field, its actual power input, one Boolean enabled channel, suppression between zero and one and a positive emission scale.');
  if(entity&&(!entity.damage||!entity.interior?.placements?.[system.id]
    ||!system.damage_component?.some(row=>row.binding==='operation')
    ||!entity.energy?.consumer?.some(row=>row.system===system.id&&row.continuous_cost_per_sec>0)))
    throw Error('Concealment requires actual placement, Operation and positive continuous energy work.');
}
