// Validate generated assets and their common coordinate system after a build.
import {readFile,writeFile} from 'node:fs/promises';
import assert from 'node:assert/strict';
import {NodeIO} from '@gltf-transform/core';
import {ALL_EXTENSIONS} from '@gltf-transform/extensions';
import {getBounds} from '@gltf-transform/functions';
import validator from 'gltf-validator';
const io=new NodeIO().registerExtensions(ALL_EXTENSIONS);
const stem='assets/models/alliance_destroyer_recreated';
const match=JSON.parse(await readFile('raw/models/PPAllianceDestroyer/recreated/size-match.json','utf8'));
const results=[];
for(const suffix of ['', '_lod1','_lod2']){
  const file=`${stem}${suffix}.glb`,bytes=await readFile(file);
  const gltf=JSON.parse(bytes.subarray(20,20+bytes.readUInt32LE(12)));
  const doc=await io.read(file),bounds=getBounds(doc.getRoot().listScenes()[0]);
  const validation=await validator.validateBytes(new Uint8Array(bytes),{uri:file,maxIssues:20});
  assert.equal(validation.issues.numErrors,0,`${file}: invalid GLB`);
  const target=match.referenceRenderedBounds;
  // Fine aerials may disappear in distant tiers. The hull footprint must stay
  // within 1 cm, and the near mesh must match all three axes exactly.
  for(const axis of suffix ? [0,2] : [0,1,2])for(const bound of ['min','max'])
    assert.ok(Math.abs(bounds[bound][axis]-target[bound][axis])<(suffix?.01:1e-6),`${file}: ${bound} axis ${axis} shifted`);
  const triangles=gltf.meshes.flatMap(m=>m.primitives).reduce((n,p)=>n+gltf.accessors[p.indices].count/3,0);
  results.push({file,triangles,bytes:bytes.length,bounds,validation:validation.issues});
}
assert.ok(results[1].triangles<results[0].triangles*.4,'Middle tier did not simplify');
assert.ok(results[2].triangles<results[1].triangles*.65,'Far tier did not simplify');
await writeFile('raw/models/PPAllianceDestroyer/recreated/lod-metrics.json',JSON.stringify(results,null,2)+'\n');
console.log(JSON.stringify(results.map(({file,triangles,bytes,validation})=>({file,triangles,bytes,errors:validation.numErrors,warnings:validation.numWarnings})),null,2));
