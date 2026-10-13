#!/usr/bin/env node
// Reuse the ratified Cruiser matrix/fold against a provenance-pinned baseline binary.
import { readFileSync, writeFileSync, existsSync, mkdirSync } from 'node:fs';
import { resolve, dirname, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { parse } from 'smol-toml';
import { expandMatchups, buildRunTasks, buildRunArgs, runFileName, mergeReports, evaluateThresholds, failedThresholds, formatMarkdown, formatThresholds } from './balance-runs.mjs';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const args=process.argv.slice(2);
if(args.length!==4 || args[0]!=='--capture' || args[2]!=='--out') throw new Error('Usage: node scripts/redesign-balance-baseline.mjs --capture <baseline-summary.json> --out .phoenix/<new-directory>');
const capture=JSON.parse(readFileSync(resolve(root,args[1]),'utf8'));
const output=resolve(root,args[3]);
const inside=relative(resolve(root,'.phoenix'),output);
if(!inside || inside.startsWith('..') || isAbsolute(inside) || existsSync(output)) throw new Error('Use a new directory inside .phoenix');
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
function git(argv){const r=spawnSync('git',argv,{cwd:root,encoding:'utf8',windowsHide:true});if(r.status!==0)throw new Error(r.stderr||'Git failed');return r.stdout.trim();}
function verify(){
  if(capture.status!=='passed' || git(['rev-parse','HEAD'])!==capture.revision || sha(readFileSync(capture.executable))!==capture.executableSha256)throw new Error('Baseline binary/revision receipt is invalid');
  if(git(['status','--porcelain','--untracked-files=all','--','Cargo.toml','Cargo.lock','build.rs','src','crates','assets','packages','gui','editor','server.html','client.html']))throw new Error('Runtime/content changed since baseline');
}
verify();mkdirSync(output,{recursive:true});
const configPath=resolve(root,'scripts/balance-runs.cruiser.toml');
const configBytes=readFileSync(configPath);const config=parse(configBytes.toString('utf8'));
const matchups=expandMatchups(config),tasks=buildRunTasks(config,matchups),runs=[];
const receipt={schema_version:1,status:'running',revision:capture.revision,profile:capture.profile,features:capture.features,runtimeSourceSha256:capture.runtimeSourceSha256,executableSha256:capture.executableSha256,config:'scripts/balance-runs.cruiser.toml',configSha256:sha(configBytes),runs:[]};
const save=()=>writeFileSync(resolve(output,'receipt.json'),JSON.stringify(receipt,null,2)+'\n');save();
try{
  for(const task of tasks){
    const argv=[...buildRunArgs(task),'--log','off'];
    const r=spawnSync(capture.executable,argv,{cwd:root,encoding:'utf8',timeout:task.timeoutSecs*1000,maxBuffer:64*1024*1024,windowsHide:true});
    if(r.error || r.status!==0)throw new Error(`${task.matchup} seed ${task.seed} failed: ${r.error?.message||r.stderr}`);
    const report=JSON.parse(r.stdout),file=runFileName(task.matchup,task.seed);
    writeFileSync(resolve(output,file),r.stdout);runs.push({...task,report});
    receipt.runs.push({matchup:task.matchup,seed:task.seed,arguments:argv,outcome:report.outcome,report:file,sha256:sha(r.stdout)});save();
    console.log(`${receipt.runs.length}/${tasks.length} ${task.matchup} seed ${task.seed}: ${report.outcome}`);
  }
  const summary=mergeReports(runs);const thresholds=evaluateThresholds(summary,matchups,config.thresholds||{});
  const failures=failedThresholds(thresholds);
  writeFileSync(resolve(output,'merged.json'),JSON.stringify({...summary,thresholds},null,2)+'\n');
  writeFileSync(resolve(output,'summary.md'),formatMarkdown(summary)+'\n\n'+formatThresholds(thresholds,{enforced:config.enforce_thresholds===true})+'\n');
  receipt.thresholds=thresholds;receipt.status=failures.length?'failed':'passed';verify();save();
  if(failures.length)throw new Error(failures.length+' ratified thresholds failed; no thresholds were changed');
  console.log('Ratified Cruiser baseline: '+runs.length+' runs, '+thresholds.length+' thresholds passed.');
}catch(error){receipt.status='failed';receipt.failure=error.message;save();throw error;}
