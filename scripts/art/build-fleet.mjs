// Author independent masters sequentially, leaving full per-model build logs.
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
const specs=JSON.parse(await readFile('scripts/art/fleet.json','utf8'));
const chosen=process.argv.slice(2);
for(const spec of specs.filter(s=>!chosen.length||chosen.includes(s.name))){
  const out=`raw/models/${spec.concept}/recreated`;
  await mkdir(out,{recursive:true});
  const env={...process.env,PHOENIX_ART_OUT:out,PHOENIX_ART_TITLE:spec.title,PHOENIX_ART_FACTION:spec.faction};
  for(const [command,args,log] of [
    [process.execPath,['scripts/art/destroyer-atlas.mjs'],'atlas.log'],
    [process.env.PHOENIX_BLENDER||'C:/Program Files/Blender Foundation/Blender 5.0/blender.exe',['-b','--factory-startup','--python','scripts/art/recreate-fleet.py','--',spec.name],'build.log'],
  ]){
    const r=spawnSync(command,args,{env,encoding:'utf8',maxBuffer:32*1024*1024,windowsHide:true});
    await writeFile(`${out}/${log}`,(r.stdout||'')+(r.stderr||''));
    if(r.error||r.status!==0||/Traceback/.test(r.stdout+r.stderr))throw new Error(`${spec.name}: ${log}: ${r.error||r.stdout+r.stderr}`);
  }
  console.log(await readFile(`${out}/metrics.json`,'utf8'));
}
