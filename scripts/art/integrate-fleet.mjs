// Swap only the entity models used by Combat Test / Falling Skyway.
import {readFile,writeFile} from 'node:fs/promises';
import assert from 'node:assert/strict';
import {parse} from 'smol-toml';
const models={
  alliance_cruiser:'alliance_cruiser',alliance_battleship:'alliance_battleship',alliance_courier:'alliance_courier',
  station_axiom:'alliance_starbase',skyhook:'alliance_starbase',depot_transfer:'alliance_research_outpost',
  ship_harrow_cruiser:'dynasty_cruiser',ship_harrow_patrol:'dynasty_cruiser',ship_harrow_destroyer:'dynasty_battleship',
  ship_civilian_hauler:'dynasty_courier',ship_requiem_courier:'dynasty_courier',umbilical_berth:'alliance_courier',
};
for(const [entity,model] of Object.entries(models)){
  const path=`assets/entities/${entity}.toml`,text=await readFile(path,'utf8'),before=parse(text);
  const stem=`${model}_recreated${entity==='umbilical_berth'?'_dock_probe':''}`;
  const replacement=`assets/models/${stem}.glb`;
  assert.ok([`assets/models/${model}.glb`,replacement].includes(before.mesh.model),entity+' unexpected existing model');
  const rig=parse(await readFile(`assets/models/${stem}.${before.mesh.variant||'model'}.toml`,'utf8'));
  for(const l of rig.lod)await readFile(l.model||l.billboard);
  const updated=text.replace(`model = "${before.mesh.model}"`,`model = "${replacement}"`);
  const after=parse(updated);after.mesh.model=before.mesh.model;
  assert.deepEqual(after,before,entity+' changed beyond its model reference');
  if(updated!==text)await writeFile(path,updated);
  console.log(`${entity} -> ${replacement}`);
}
