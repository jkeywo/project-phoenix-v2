// Validate the actual deliverable and report comparable asset costs.
import { readFile, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
import validator from 'gltf-validator';
import sharp from 'sharp';
const out='raw/models/PPAllianceDestroyer/recreated';
const candidate=`${out}/alliance_destroyer_recreated.glb`;
const files=['raw/models/PPAllianceDestroyer/base_basic_pbr.glb','assets/models/alliance_destroyer.glb',candidate];
const inventory=[];
for (const file of files) {
  const bytes=await readFile(file), jsonLength=bytes.readUInt32LE(12);
  const gltf=JSON.parse(bytes.subarray(20,20+jsonLength).toString());
  const binOffset=jsonLength+28;
  const images=[];
  for (const image of gltf.images) {
    const view=gltf.bufferViews[image.bufferView];
    const start=binOffset+(view.byteOffset??0);
    const metadata=await sharp(bytes.subarray(start,start+view.byteLength)).metadata();
    images.push({name:image.name,width:metadata.width,height:metadata.height});
  }
  const primitives=gltf.meshes.flatMap(m=>m.primitives);
  const record={file,bytes:bytes.length,triangles:primitives.reduce((sum,p)=>sum+gltf.accessors[p.indices].count/3,0),primitives:primitives.length,materials:gltf.materials.length,texturePixels:images.reduce((sum,i)=>sum+i.width*i.height,0),images};
  inventory.push(record);
  if (file===candidate) {
    const report=await validator.validateBytes(new Uint8Array(bytes),{uri:file,maxIssues:100});
    await writeFile(`${out}/validation.json`,JSON.stringify(report,null,2)+'\n');
    assert.equal(report.issues.numErrors,0,'glTF validation errors');
    assert.equal(report.issues.numWarnings,0,'glTF validation warnings');
    assert.equal(primitives.length,1,'One render primitive expected');
    assert.equal(gltf.materials.length,1,'One material expected');
    // Revision 2 spends more geometry on curved armour and more texels on
    // panel detail in response to the user's visual review of the first pass.
    assert.ok(record.triangles<24000,'Near mesh triangle budget');
    assert.ok(record.texturePixels<6000000,'Texture pixel budget');
    assert.ok(record.bytes<3000000,'GLB byte budget');
    assert.ok(primitives[0].attributes.TANGENT!==undefined,'Tangents must be precomputed');
    assert.ok(gltf.materials.every(m=>!m.doubleSided),'Closed surfaces must cull back faces');
    assert.ok(gltf.images.every(i=>i.bufferView!==undefined&&!i.uri),'Self-contained textures');
    assert.ok(gltf.buffers.every(b=>!b.uri),'Self-contained mesh');
  }
}
await writeFile(`${out}/comparison.json`,JSON.stringify(inventory,null,2)+'\n');
const [raw,shipped,newModel]=inventory;
const reduction=(key)=>((1-newModel[key]/shipped[key])*100).toFixed(1)+'%';
console.log(JSON.stringify({validation:'0 errors, 0 warnings',triangles:newModel.triangles,bytes:newModel.bytes,texturePixels:newModel.texturePixels,reductionsVsShipped:{triangles:reduction('triangles'),bytes:reduction('bytes'),texturePixels:reduction('texturePixels')}},null,2));
