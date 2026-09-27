#!/usr/bin/env node
// Six real native processes. Direct WebRTC is not a native transport route.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseNativeProbeArgs } from './fleet-native-probe.mjs';
import { runNativeAdmissionMatrix } from './fleet-native-matrix.mjs';
import { mixedFailureHook } from './fleet-mixed-recovery.mjs';
import { nativeGmRedialHook } from './fleet-native-redial.mjs';
import { sourceState } from './fleet-wasm-build-receipt.mjs';
import { fileHash } from './profile-provenance.mjs';

export function nativeRecoveryOptions(args) {
  const copy=[...args];
  const take=name=>{
    const indexes=copy.flatMap((value,index)=>value===name?[index]:[]);
    if(indexes.length!==1)throw new Error('Exactly one '+name+' is required');
    const index=indexes[0],value=copy[index+1];
    if(!value||value.startsWith('--'))throw new Error('Incomplete '+name);
    copy.splice(index,2);return value;
  };
  const failure=take('--failure'),receipt=take('--build-receipt');
  if(!['ship','gm','leader','replacement','divergence','gm-redial'].includes(failure))throw new Error('Invalid native recovery scenario');
  const options=parseNativeProbeArgs(copy);
  if(!options.workload)throw new Error('Native recovery requires --workload true');
  return {failure,receipt,options};
}
export async function main(args=process.argv.slice(2)) {
  const {failure,receipt:file,options}=nativeRecoveryOptions(args);
  const receipt=JSON.parse(fs.readFileSync(file,'utf8')),source=sourceState(options.source);
  if(source.sourcePatch||receipt.dirtyDiff?.length||receipt.sourceRevision!==source.sourceRevision
    ||receipt.binarySha256!==fileHash(options.binary)||receipt.libraryMtimeRefreshedWithoutByteChange!==true)
    throw new Error('Native build receipt does not match clean source and binary');
  const result=await runNativeAdmissionMatrix(options,{afterHealthy:failure==='gm-redial'?nativeGmRedialHook():mixedFailureHook(failure,{
    faultSeconds:600,nativeLabels:['ship-1','ship-2','ship-3','ship-4','gm-1','gm-2'],
  })});
  result.buildReceipt={path:path.resolve(file),sha256:fileHash(file),sourceRevision:source.sourceRevision};
  result.kind='native-six-peer-recovery';
  fs.writeFileSync(path.join(options.out,'matrix.json'),JSON.stringify(result,null,2)+'\n');
  return result;
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url))main().then(result=>{
  console.log(JSON.stringify({failure:result.failure,recovery:result.recovery?.outcome,passed:result.recoveryPassed},null,2));
  process.exitCode=result.recoveryPassed?0:1;
},error=>{console.error(error);process.exitCode=1;});
