export {initialIdentity, validateIdentityComposition} from './workshop-identity.js';
import {validateChannelInitials} from './workshop-protection.js';
export {initialProtection, validateProtectionComposition} from './workshop-protection.js';
import { parse, stringify } from 'smol-toml';

export const turretKinds = ['phaser_bank', 'blaster_bank', 'torpedo_tube', 'passive_receiver'];
export const initialTurret = () => [
  { kind: 'power_input', id: 'turret-power' },
  { kind: 'turret', id: 'turret', power_input: 'turret-power', config: {
    facing_deg: 0, traverse_arc_deg: 270, traverse_deg_per_sec: 45, aim_tolerance_deg: 2,
  } },
];

export const initialIllumination = () => ({
  components: [{kind:'power_input',id:'input'}, {kind:'illumination_emitter',id:'emitter',power_input:'input',enabled_channel:'enabled',target_channel:'target',config:{mount:{facing_deg:0,fire_arc_deg:60,range:200},link_range:200,minimum_transmission:0.1,reading_ticks:30}}],
  channels: [{name:'enabled',kind:'boolean'},{name:'target',kind:'entity_target'}],
});
export const initialJamming = () => ({
  components: [{kind:'power_input',id:'input'}, {kind:'jamming_emitter',id:'emitter',power_input:'input',enabled_channel:'enabled',target_channel:'target',config:{mount:{facing_deg:0,fire_arc_deg:60,range:200},falloff:'linear',effect:'comms',strength:4,maximum_sample_age_secs:2,reading_ticks:30}}],
  channels: [{name:'enabled',kind:'boolean'},{name:'target',kind:'entity_target'}],
});
const finiteEmitterNumber=value=>typeof value==='number'&&Number.isFinite(Math.fround(value));
const positiveEmitterNumber=value=>finiteEmitterNumber(value)&&Math.fround(value)>0;
export function validateIlluminationComposition(system, components, channels) {
  if(system.kind!=='illumination')return;
  validateMountedEmitter(system,components,channels,'illumination_emitter',config=>
    Object.keys(config).every(key=>['mount','link_range','minimum_transmission','reading_ticks'].includes(key))
    && positiveEmitterNumber(config.link_range)&&positiveEmitterNumber(config.minimum_transmission)&&config.minimum_transmission<=1);
}
export function validateJammingComposition(system, components, channels) {
  if(system.kind!=='jamming')return;
  validateMountedEmitter(system,components,channels,'jamming_emitter',config=>
    Object.keys(config).every(key=>['mount','falloff','strength','maximum_sample_age_secs','reading_ticks','effect'].includes(key))
    && [undefined,'comms','sensors'].includes(config.effect) && ['step','linear','smooth'].includes(config.falloff)&&positiveEmitterNumber(config.strength)&&positiveEmitterNumber(config.maximum_sample_age_secs));
}
function validateMountedEmitter(system,components,channels,emitterKind,validProfile) {
  const emitter=components.find(row=>row.kind===emitterKind), config=emitter?.config, mount=config?.mount;
  const id=value=>typeof value==='string'&&/^[a-z0-9-]+$/.test(value);
  if(!system.power_group || !emitter || channels.length!==2 || components.filter(row=>row.kind===emitterKind).length!==1
    || !components.every(row=>id(row.id)) || new Set(components.map(row=>row.id)).size!==components.length
    || components.filter(row=>row.kind==='power_input'&&row.id===emitter.power_input).length!==1
    || components.some(row=>![emitterKind,'power_input','turret'].includes(row.kind))
    || components.filter(row=>row.kind==='power_input').length!==1 || components.filter(row=>row.kind==='turret').length>1
    || channels.some(row=>!id(row.name)) || new Set(channels.map(row=>row.name)).size!==2
    || !channels.some(row=>row.name===emitter.enabled_channel&&row.kind==='boolean')
    || !channels.some(row=>row.name===emitter.target_channel&&row.kind==='entity_target')
    || !mount || !config || !validProfile(config)
    || Object.keys(mount).some(key=>!['facing_deg','fire_arc_deg','range'].includes(key))
    || ![mount.facing_deg,mount.fire_arc_deg,mount.range].every(finiteEmitterNumber)
    || Math.abs(mount.facing_deg)>180 || Math.fround(mount.fire_arc_deg)<=0 || mount.fire_arc_deg>360 || Math.fround(mount.range)<=0
    || !Number.isInteger(config.reading_ticks) || config.reading_ticks<0 || config.reading_ticks>4294967295
    || Object.keys(emitter).some(key=>!['id','kind','enabled_channel','target_channel','power_input','config'].includes(key))
    || components.some(row=>row.kind==='power_input'&&Object.keys(row).some(key=>!['id','kind'].includes(key))))
    throw new Error('Use one finite emitter, one actual power input and exactly its enabled/target channels.');
  if(components.some(row=>row.kind==='turret'))validateTurretComposition({...system,kind:'phaser_bank'},components.filter(row=>row.kind!==emitterKind),[]);
}

export function validateTurretComposition(system, components, channels) {
  if (!turretKinds.includes(system.kind) || !components.length && !channels.length) return;
  const turret = components.find(row => row.kind === 'turret');
  const id = value => typeof value === 'string' && /^[a-z0-9-]+$/.test(value);
  const finite = value => typeof value === 'number' && Number.isFinite(Math.fround(value));
  const config = turret?.config;
  if (!system.power_group || channels.length || components.length !== 2 || !turret
    || components.some(row => !id(row.id)) || new Set(components.map(row => row.id)).size !== 2
    || !components.some(row => row.kind === 'power_input' && row.id === turret.power_input)
    || Object.keys(turret).some(key => !['kind', 'id', 'power_input', 'config'].includes(key))
    || !config || Object.keys(config).length !== 4
    || !['facing_deg', 'traverse_arc_deg', 'traverse_deg_per_sec', 'aim_tolerance_deg'].every(key => finite(config[key]))
    || Math.abs(config.facing_deg) > 180 || Math.fround(config.traverse_arc_deg) <= 0 || config.traverse_arc_deg > 360
    || Math.fround(config.traverse_deg_per_sec) <= 0 || config.aim_tolerance_deg < 0 || config.aim_tolerance_deg > Math.fround(config.traverse_arc_deg) * 0.5
    || components.some(row => row.kind === 'power_input' && Object.keys(row).some(key => !['kind', 'id'].includes(key)))) {
    throw new Error('Use one finite turret and its power input on the owning System, without additional command channels.');
  }
}

export function validateDockMovement(components) {
  for (const row of components) if (row.kind === 'dock_mating' && row.config?.moves_body !== undefined && typeof row.config.moves_body !== 'boolean')
    throw new Error('Dock moves_body must be a supported boolean; stationary berths default false.');
}

/** Replace only the selected System's composition, as one accepted transaction.
 * Nested identities are scoped to that System; inherited data requires an
 * explicit materialisation action, never a write into its supplying include. */
export function writeSystemComposition(source, id, operation, effective) {
  const headers = [...source.matchAll(/^[ \t]*\[(\[?)([^\]\r\n]+)\]\]?[^\r\n]*(?:\r?\n|$)/gm)]
    .map(match => ({ name: match[2].trim(), array: match[1] === '[', start: match.index, end: match.index + match[0].length }));
  const start = headers.findIndex((header, index) => {
    if (header.name !== 'system' || !header.array) return false;
    const end = headers.slice(index + 1).find(next => next.name === 'system' || !next.name.startsWith('system.'))?.start ?? source.length;
    return parse(source.slice(header.start, end)).system?.[0]?.id === id;
  });
  if (start < 0) throw new Error('Materialise the containing System before editing its components.');
  const first = headers[start];
  const end = headers.slice(start + 1).find(next => next.name === 'system' || !next.name.startsWith('system.'))?.start ?? source.length;
  const local = parse(source.slice(first.start, end)).system[0];
  const components = structuredClone(operation.components || []), channels = structuredClone(operation.channels || []);
  validateChannelInitials(channels);
  for (const [key, identity, next] of [['component', 'id', components], ['channel', 'name', channels]]) {
    const inherited = (effective[key] || []).filter(row => !(local[key] || []).some(own => own[identity] === row[identity]
      && !own._remove && JSON.stringify(own) === JSON.stringify(row)));
    if (inherited.length && !operation.materialise) throw new Error('Materialise inherited components and channels explicitly before editing this composition.');
    // Reconciled inherited entries retain their include order. Do not promise
    // an order that the runtime merge will silently ignore.
    const kept = inherited.filter(row => next.some(item => item[identity] === row[identity]));
    if (kept.map(row => row[identity]).join('\0') !== next.filter(row => kept.some(item => item[identity] === row[identity])).map(row => row[identity]).join('\0')) {
      throw new Error('Inherited composition order is defined by its source include.');
    }
    for (const row of inherited) if (!next.some(item => item[identity] === row[identity])) next.push({ [identity]: row[identity], _remove: true });
  }
  let body = source.slice(first.start, end);
  const remove = headers.filter(header => header.start > first.start && header.start < end
    && ['system.component', 'system.channel'].includes(header.name));
  for (const header of remove.reverse()) {
    const stop = headers.find(next => next.start > header.start && !next.name.startsWith(`${header.name}.`))?.start ?? source.length;
    body = body.slice(0, header.start - first.start) + body.slice(Math.min(stop, end) - first.start);
  }
  const nl = source.includes('\r\n') ? '\r\n' : '\n';
  const serialized = stringify({ system: [{ component: components, ...(channels.length ? { channel: channels } : {}) }] })
    .replace(/^\[\[system\]\]\r?\n/, '').trim().replace(/\n/g, nl);
  return source.slice(0, first.start) + body.trimEnd() + nl + nl + serialized + nl + nl + source.slice(end);
}

export const initialBay=()=>({components:[{kind:'power_input',id:'input'},{kind:'craft_bay',id:'bay',power_input:'input',config:{compartment:'bay',capacity_space:2,launch_work_secs:2,recovery_work_secs:3,recovery_range:20,maximum_parent_speed:0.1,release_offset:[0,0,-18]}}],channels:[]});
export function validateBayComposition(system,components,channels){
 if(system.kind!=='craft-bay')return;
 const bay=components.find(row=>row.kind==='craft_bay'),input=components.find(row=>row.kind==='power_input'),config=bay?.config;
 const finite=value=>typeof value==='number'&&Number.isFinite(Math.fround(value));
 const positive=value=>finite(value)&&Math.fround(value)>0;
 if(!system.power_group||channels.length||components.length!==2||!bay||!input||bay.id===input.id||bay.power_input!==input.id
  ||components.some(row=>!row.id||Object.keys(row).some(key=>!(['kind','id',...(row===bay?['power_input','config']:[])].includes(key))))
  ||!config||Object.keys(config).some(key=>!['compartment','capacity_space','launch_work_secs','recovery_work_secs','recovery_range','maximum_parent_speed','release_offset'].includes(key))
  ||typeof config.compartment!=='string'||!config.compartment.trim()||!Number.isSafeInteger(config.capacity_space)||config.capacity_space<=0
  ||!['launch_work_secs','recovery_work_secs','recovery_range'].every(key=>positive(config[key]))
  ||!finite(config.maximum_parent_speed)||config.maximum_parent_speed<0||!Array.isArray(config.release_offset)||config.release_offset.length!==3||!config.release_offset.every(finite))
  throw new Error('Craft Bay needs one supported finite handling component, actual power input and dedicated compartment; it has no extra command channels.');
}
