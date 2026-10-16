export function validateLoaderGroups(entity,groups=entity?.torpedoes?.loader_groups??[]) {
  if(!Array.isArray(groups))throw new Error('Loader groups require a list.');
  const tubes=new Set((entity?.torpedoes?.tubes??[]).map(row=>row.id));
  for(const weapon of entity?.weapon??[])if(weapon.delivery?.kind==='guided')tubes.add(weapon.strategy_id||weapon.id);
  const ids=new Set(),members=new Set();
  for(const group of groups){
    if(!group||Object.keys(group).some(key=>!['id','tubes','work_rate'].includes(key))||typeof group.id!=='string'||!/^[a-z0-9-]{1,115}$/.test(group.id)||ids.has(group.id)
      ||!Array.isArray(group.tubes)||group.tubes.length<2||!Number.isFinite(Math.fround(group.work_rate))||Math.fround(group.work_rate)<=0||group.work_rate>group.tubes.length)throw new Error('Choose unique group ids, at least two guided tubes and a finite positive shared rate.');
    ids.add(group.id);
    for(const tube of group.tubes){if(!tubes.has(tube)||members.has(tube))throw new Error('Loader member is missing, duplicated or belongs to another group.');members.add(tube);}
  }
  return groups;
}
export function guardLoaderReferences(_before,after){validateLoaderGroups(after);}
