// Inspect exported bytes, budgets, fitted bounds, attachments and LOD coverage.
import {readFile,writeFile} from 'node:fs/promises';
import assert from 'node:assert/strict';
import {NodeIO} from '@gltf-transform/core';
import {ALL_EXTENSIONS} from '@gltf-transform/extensions';
import {getBounds} from '@gltf-transform/functions';
import {parse} from 'smol-toml';
import validator from 'gltf-validator';
const io=new NodeIO().registerExtensions(ALL_EXTENSIONS);
const jobs=JSON.parse(await readFile('scripts/art/fleet-fit.json','utf8'));
const report=[];
function quaternionPoint(v,base,direction=false){
  const [x,y,z]=v.map((n,i)=>n*base.scale[i]);
  const q=Math.sin(base.rotation[1]/2),w=Math.cos(base.rotation[1]/2);
  const out=[x+2*w*q*z-2*q*q*x,y,z-2*w*q*x-2*q*q*z];
  return direction?out.map(n=>n/Math.hypot(...out)):out.map((n,i)=>n+base.offset[i]);
}
const equal=(a,b)=>a.length===b.length&&a.every((n,i)=>Math.abs(n-b[i])<1e-6);
for(const job of jobs){
  const rig=parse(await readFile(`${job.candidate}.${job.variant}.toml`,'utf8'));
  const before=job.originalRig;
  assert.deepEqual(Object.keys(rig.markers||{}).sort(),Object.keys(before.markers||{}).sort());
  for(const [name,m] of Object.entries(before.markers||{})){
    assert.ok(equal(rig.markers[name].position,quaternionPoint(m.position,before.base)),`${job.name}: ${name} moved`);
    assert.ok(equal(rig.markers[name].direction,quaternionPoint(m.direction,before.base,true)),`${job.name}: ${name} direction changed`);
  }
  assert.equal(rig.target_points?.length||0,before.target_points?.length||0);
  for(const [i,p] of (before.target_points||[]).entries())assert.ok(equal(rig.target_points[i].position,quaternionPoint(p.position,before.base)));
  const entries=[];
  for(const [index,level] of rig.lod.entries()){
    if(!level.model)continue;
    const bytes=await readFile(level.model);
    const gltf=JSON.parse(bytes.subarray(20,20+bytes.readUInt32LE(12)));
    const check=await validator.validateBytes(new Uint8Array(bytes),{uri:level.model,maxIssues:30});
    assert.equal(check.issues.numErrors,0,`${level.model}: ${JSON.stringify(check.issues)}`);
    const primitives=gltf.meshes.flatMap(m=>m.primitives);
    assert.equal(primitives.length,1);assert.equal(gltf.materials.length,1);
    assert.ok(gltf.images.every(i=>i.bufferView!==undefined&&!i.uri));
    assert.ok(gltf.materials.every(m=>!m.doubleSided));
    if(index===0)assert.ok(primitives[0].attributes.TANGENT!==undefined);
    const triangles=primitives.reduce((n,p)=>n+gltf.accessors[p.indices].count/3,0);
    const doc=await io.read(level.model),bounds=getBounds(doc.getRoot().listScenes()[0]);
    for(const side of ['min','max'])for(const axis of index===0?[0,1,2]:[0,2]){
      const tolerance=index===0?1e-6:(job.bounds.max[axis]-job.bounds.min[axis])*.07;
      assert.ok(Math.abs(bounds[side][axis]-job.bounds[side][axis])<tolerance,`${level.model}: silhouette bounds drift`);
    }
    entries.push({file:level.model,triangles,bytes:bytes.length,validationErrors:check.issues.numErrors,validationWarnings:check.issues.numWarnings});
  }
  assert.ok(entries[0].triangles<24000,job.name+' near budget');
  assert.ok(entries[1].triangles<entries[0].triangles*.72,job.name+' middle budget');
  assert.ok(entries[2].triangles<entries[1].triangles*.85,job.name+' far budget');
  const originalBytes=await readFile(`assets/models/${job.name}.glb`);
  const original=JSON.parse(originalBytes.subarray(20,20+originalBytes.readUInt32LE(12)));
  const oldTriangles=original.meshes.flatMap(m=>m.primitives).reduce((n,p)=>n+original.accessors[p.indices].count/3,0);
  assert.ok(entries[0].triangles<oldTriangles,job.name+' triangle improvement');
  assert.ok(entries[0].bytes<originalBytes.length,job.name+' byte improvement');
  report.push({model:job.name,variant:job.variant,originalTriangles:oldTriangles,originalBytes:originalBytes.length,levels:entries,markers:Object.keys(rig.markers||{}).length,targetPoints:rig.target_points?.length||0});
}
await writeFile('scripts/art/fleet-validation.json',JSON.stringify(report,null,2)+'\n');
for(const r of report)console.log(`${r.model} (${r.variant}): ${r.originalTriangles} -> ${r.levels.map(l=>l.triangles).join(' / ')} triangles; ${r.markers} markers, ${r.targetPoints} target points`);
