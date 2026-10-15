import {validateChannelInitials} from './workshop-protection.js';
export const initialIdentity = () => ({
  components:[{kind:'power_input',id:'input'},{kind:'identity_emitter',id:'transponder',power_input:'input',enabled_channel:'enabled',config:{callsign:'Transit courier'}}],
  channels:[{name:'enabled',kind:'boolean',initial:{kind:'boolean',value:false}}],
});
export function validateIdentityComposition(system,components,channels,entity=null) {
  if(system.kind!=='identity-emitter')return;
  validateChannelInitials(channels);
  const emitter=components.find(row=>row.kind==='identity_emitter'),config=emitter?.config;
  const id=value=>typeof value==='string'&&/^[a-z0-9-]+$/.test(value);
  if(!system.power_group || components.length!==2 || !emitter || !components.every(row=>id(row.id))
    || new Set(components.map(row=>row.id)).size!==2
    || components.filter(row=>row.kind==='power_input'&&row.id===emitter.power_input).length!==1
    || Object.keys(emitter).some(key=>!['kind','id','power_input','enabled_channel','config'].includes(key))
    || components.some(row=>row.kind==='power_input'&&Object.keys(row).some(key=>!['kind','id'].includes(key)))
    || channels.length!==1 || !id(channels[0]?.name) || channels[0].name!==emitter.enabled_channel || channels[0].kind!=='boolean'
    || !config || Object.keys(config).some(key=>!['callsign','faction'].includes(key))
    || typeof config.callsign!=='string' || !config.callsign.trim() || new TextEncoder().encode(config.callsign).length>128
    || /[\u0000-\u001f\u007f-\u009f]/u.test(config.callsign)
    || config.faction!=null && (typeof config.faction!=='string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu.test(config.faction)))
    throw Error('Use one identity emitter, its distinct actual power input, one Boolean enabled channel and a bounded literal callsign.');
  if(entity && (!entity.damage || !entity.interior?.placements?.[system.id]
    || !system.damage_component?.some(row=>row.binding==='operation')
    || !entity.energy?.consumer?.some(row=>row.system===system.id&&row.continuous_cost_per_sec>0)))
    throw Error('Identity emission requires real placement, Operation and positive continuous energy work.');
}
