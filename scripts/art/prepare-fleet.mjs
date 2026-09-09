// Fit meshes to actual original GLB vertices, preserving rig-local gameplay.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {NodeIO} from '@gltf-transform/core';
import {ALL_EXTENSIONS} from '@gltf-transform/extensions';
import {getBounds} from '@gltf-transform/functions';
import {parse,stringify} from 'smol-toml';
import assert from 'node:assert/strict';
import {migrateMarkers} from './destroyer-markers.mjs';
const specs=JSON.parse(await readFile('scripts/art/fleet.json','utf8'));
const chosen=process.argv.slice(2);
for(const name of chosen)assert.ok(specs.some(s=>s.name===name),'unknown fleet model: '+name);
const previous=chosen.length?JSON.parse(await readFile('scripts/art/fleet-fit.json','utf8')):[];
const io=new NodeIO().registerExtensions(ALL_EXTENSIONS);
const identity={offset:[0,0,0],rotation:[0,0,0],scale:[1,1,1]};
const size=b=>b.max.map((v,i)=>v-b.min[i]);
const centre=b=>b.max.map((v,i)=>(v+b.min[i])/2);
function parent(doc,base){
  assert.equal(base.rotation[0],0);assert.equal(base.rotation[2],0);
  const scene=doc.getRoot().listScenes()[0],node=doc.createNode('Baked original rig frame');
  for(const child of [...scene.listChildren()])node.addChild(child);
  scene.addChild(node);
  node.setTranslation(base.offset).setScale(base.scale).setRotation([0,Math.sin(base.rotation[1]/2),0,Math.cos(base.rotation[1]/2)]);
  return {scene,node};
}
function point(p,base){
  const [x,y,z]=p.map((n,i)=>n*base.scale[i]),c=Math.cos(base.rotation[1]),s=Math.sin(base.rotation[1]);
  return [c*x+s*z,y,-s*x+c*z].map((n,i)=>n+base.offset[i]);
}
await mkdir('scripts/art/lod-sources',{recursive:true});
const jobs=specs.map(s=>({...s,variant:'model'}));
jobs.push({...specs.find(s=>s.name==='alliance_courier'),variant:'dock_probe'});
const reports=[];
for(const spec of jobs){
  if(chosen.length&&!chosen.includes(spec.name)){
    const report=previous.find(r=>r.name===spec.name&&r.variant===spec.variant);
    assert.ok(report,'run full preparation once before a subset build');
    reports.push(report);continue;
  }
  const original=`assets/models/${spec.name}`;
  const reference=parse(await readFile(`${original}.${spec.variant}.toml`,'utf8'));
  const candidate=`assets/models/${spec.name}_recreated${spec.variant==='model'?'':'_'+spec.variant}`;
  const out=`raw/models/${spec.concept}/recreated`;
  const target=getBounds(parent(await io.read(`${original}.glb`),reference.base).scene);
  const master=await io.read(`${out}/${spec.name}_recreated.glb`);
  const raw=getBounds(master.getRoot().listScenes()[0]);
  const base={offset:[0,0,0],rotation:reference.base.rotation,scale:size(raw).map((v,i)=>size(target)[i]/v)};
  const current=parent(master,base);
  for(let pass=0;pass<16;pass++){
    const actual=getBounds(current.scene);
    base.scale=base.scale.map((s,i)=>s*size(target)[i]/size(actual)[i]);current.node.setScale(base.scale);
    const c=centre(getBounds(current.scene));base.offset=base.offset.map((v,i)=>v+centre(target)[i]-c[i]);current.node.setTranslation(base.offset);
  }
  const bounds=getBounds(current.scene);
  for(const side of ['min','max'])assert.ok(bounds[side].every((v,i)=>Math.abs(v-target[side][i])<1e-6),spec.name+' size mismatch');
  await io.write(`${candidate}.glb`,master);
  const coarse=await io.read(`${out}/${spec.name}_lod_source.glb`);
  parent(coarse,base);
  const source=`scripts/art/lod-sources/${spec.name}${spec.variant==='model'?'':'_'+spec.variant}.glb`;
  await io.write(source,coarse);
  const far=await io.read(`${out}/${spec.name}_far_source.glb`);
  parent(far,base);
  const farSource=source.replace('.glb','_far.glb');
  await io.write(farSource,far);
  const lod=reference.lod.map((old,i)=>{
    const level=structuredClone(old);
    if(level.model){
      level.model=i===0?`${candidate}.glb`:`${candidate}_lod${i}.glb`;
      if(i>0){
        level.tier_rig='identity';
        level.generate={source:i===1?source:farSource,ratio:i===1?.65:.23,error:i===1?.025:.08,texture_size:i===1?256:128};
        if(spec.name==='alliance_starbase')level.generate.error=i===1?.0005:.001;
        // The open crescent is the Dynasty silhouette; the old 8% far error
        // flattened it into a straight sail even when its bounds still matched.
        if(spec.faction==='dynasty')level.generate.error=i===1?.0015:.003;
      }
    }
    if(level.billboard){level.billboard=`${candidate}_lod3.png`;level.capture.source=`${candidate}.glb`;}
    return level;
  });
  const rig={base:identity,extents:{...bounds,size:size(bounds)}};
  if(reference.markers)rig.markers=migrateMarkers(reference);
  if(reference.target_points)rig.target_points=reference.target_points.map(p=>({...p,position:point(p.position,reference.base)}));
  rig.lod=lod;
  await writeFile(`${candidate}.${spec.variant}.toml`,'# Concept recreation. Original gameplay points are baked into the identity rig frame.\n# Regenerate with scripts/art/prepare-fleet.mjs, then the standard LOD/capture tools.\n'+stringify(rig));
  const report={name:spec.name,variant:spec.variant,candidate,source,concept:spec.concept,originalBounds:target,bounds,base,originalRig:reference,rig};
  reports.push(report);
  console.log(spec.name,spec.variant,JSON.stringify(size(bounds)));
}
await writeFile('scripts/art/fleet-fit.json',JSON.stringify(reports,null,2)+'\n');
