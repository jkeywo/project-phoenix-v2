#!/usr/bin/env node
// Headless migration evidence, using the existing runner and vellum-perf capture.
// This diagnostic accepts documentation drafts; it is not clean-release profiling.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { cpus, platform, release, totalmem } from 'node:os';
import { resolve, relative, sep, dirname, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { summarizeSamples } from './profile-analysis.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
if (![2,4].includes(args.length) || args[0] !== '--out' || (args.length===4 && (args[2]!=='--profile' || !['measure','release'].includes(args[3])))) throw new Error('Usage: node scripts/redesign-baseline.mjs --out .phoenix/redesign-baseline/<new-run> [--profile measure|release]');
const profile = args[3] || 'measure';
const output = resolve(root, args[1]);
const relativeOutput = relative(resolve(root, '.phoenix'), output);
if (!relativeOutput || relativeOutput.startsWith('..') || isAbsolute(relativeOutput) || existsSync(output)) throw new Error('Use a new directory inside .phoenix');
const runtimePaths = ['Cargo.toml','Cargo.lock','build.rs','src','crates','assets','packages','gui','editor','server.html','client.html'];
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
function command(executable, argv, timeout = 600000) {
  const result = spawnSync(executable, argv, { cwd: root, encoding: 'utf8', timeout, maxBuffer: 64 * 1024 * 1024, windowsHide: true });
  if (result.error || result.status !== 0) throw new Error(`${executable} ${argv.join(' ')} failed (${result.status}): ${result.error?.message || result.stderr || result.stdout}`);
  return result.stdout;
}
function sourceState() {
  const files = command('git', ['ls-files','--cached','--others','--exclude-standard','--',...runtimePaths]).trim().split(/\r?\n/).filter(Boolean).sort();
  const hash = createHash('sha256');
  for (const path of files) { hash.update(path); hash.update('\0'); hash.update(readFileSync(resolve(root,path))); hash.update('\0'); }
  return { revision: command('git',['rev-parse','HEAD']).trim(), runtimeSourceSha256: hash.digest('hex'), runtimeFiles: files.length };
}
const before = sourceState();
const runtimeDiff = command('git',['status','--porcelain','--untracked-files=all','--',...runtimePaths]);
if (runtimeDiff.trim()) throw new Error('Capture the pre-migration baseline before changing runtime/content sources:\n' + runtimeDiff);
mkdirSync(output, { recursive: true });
const summary = {
  schema_version: 1, status: 'running', started_utc: new Date().toISOString(),
  ...before, repositoryDirty: !!command('git',['status','--porcelain','--untracked-files=all']).trim(),
  runtimeSourceClean: true, profile, requestedFeatures: ['headless'], defaultFeatures: true,
  hardware: { platform: platform(), release: release(), cpu: cpus()[0]?.model, logicalCpus: cpus().length, totalMemoryBytes: totalmem() },
  scope: 'Headless same-machine diagnostic; no browser/native rendering, real-network or human workload acceptance.',
  referenceHull: 'assets/entities/alliance_cruiser.toml', runs: [], checks: [],
};
const summaryPath = resolve(output,'summary.json');
const save = () => writeFileSync(summaryPath, JSON.stringify(summary,null,2)+'\n');
save();
try {
  console.log(`Building the current headless ${profile} executable for baseline provenance…`);
  const buildArguments = ['build','--profile',profile,'--features','headless','--bin','phoenix-headless','--message-format=json'];
  const cargoOutput = command(process.platform === 'win32' ? 'cargo.exe' : 'cargo', buildArguments, 7200000);
  writeFileSync(resolve(output,'build.jsonl'),cargoOutput);
  const artifact = cargoOutput.split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line)).findLast(m => m.reason === 'compiler-artifact' && m.target?.name === 'phoenix-headless' && m.executable);
  if (!artifact) throw new Error('Cargo supplied no headless executable artifact');
  summary.features = artifact.features;
  const binary = resolve(output,process.platform === 'win32' ? 'phoenix-headless.exe' : 'phoenix-headless');
  copyFileSync(artifact.executable,binary);
  summary.buildArtifact = artifact.executable;
  summary.executable = binary;
  summary.executableSha256 = sha(readFileSync(binary));
  summary.buildCommand = ['cargo',...buildArguments];
  if (JSON.stringify(sourceState()) !== JSON.stringify(before)) throw new Error('Runtime sources or revision changed during build');
  save();
  const fileReceipt = path => ({ path: relative(output,path).split(sep).join('/'), sha256: sha(readFileSync(path)) });
  function run(name, argv) {
    console.log('Capturing ' + name);
    const started = performance.now();
    const stdout = command(binary,argv);
    const record = { name, arguments: argv, exitCode: 0, wallSeconds: (performance.now()-started)/1000 };
    const stdoutPath = resolve(output,name+'.stdout.txt'); writeFileSync(stdoutPath,stdout);
    record.stdout = fileReceipt(stdoutPath); summary.runs.push(record); save(); return record;
  }
  for (const world of ['combat_test','falling_skyway']) {
    const common = ['--world',`assets/worlds/${world}.toml`,'--ship',summary.referenceHull,'--seed','42','--sim-seconds','60','--log','off'];
    const reports = [];
    for (let i=0;i<2;i++) {
      const path=resolve(output,`${world}-repeat-${i}.json`);
      const record=run(`${world}-repeat-${i}`,[...common,'--report',path]);
      record.report=fileReceipt(path); reports.push(record.report.sha256); save();
    }
    if (reports[0] !== reports[1]) throw new Error(world+' seeded reports differ');
    summary.checks.push({ name: world+' byte-identical seeded reports', passed: true, reportSha256: reports[0] }); save();
    const replayPath=resolve(output,world+'.replay.ron');
    const recorded=run(world+'-record',[...common,'--record',replayPath,'--digest-every','60']);
    recorded.artifact=fileReceipt(replayPath); save();
    run(world+'-replay',['--replay',replayPath,'--log','off']);
    summary.checks.push({ name: world+' replay checkpoints', passed: true }); save();
    const continuations = [];
    for (let i=0;i<3;i++) {
      const capture=resolve(output,`${world}-perf-${i}.json`);
      const report=resolve(output,`${world}-perf-${i}-report.json`);
      const record=run(`${world}-perf-${i}`,[...common,'--perf-capture',capture,'--perf-scenario','redesign-'+world,'--report',report]);
      record.capture=fileReceipt(capture); record.report=fileReceipt(report);
      const continuationPath = capture+'.continuation.json';
      record.continuation = { ...fileReceipt(continuationPath), ...JSON.parse(readFileSync(continuationPath,'utf8')) };
      continuations.push(`${record.continuation.tick}:${record.continuation.digest}`);
      const captured=JSON.parse(readFileSync(capture,'utf8'));
      const series=captured.series || captured.capture?.series;
      if (!series || !Object.keys(series).length) throw new Error('No performance series in '+capture);
      record.metrics=Object.fromEntries(Object.entries(series).filter(([,v])=>v.unit==='millis'&&Array.isArray(v.samples)&&v.samples.length).map(([key,v])=>{
        const excludedSamples=key==='sim.tick'?300:0;
        return [key,{unit:v.unit,excludedSamples,...summarizeSamples(v.samples.slice(excludedSamples))}];
      }));
      record.runSeconds=series['sim.run']?.samples?.[0] ?? null;
      if (!record.metrics['sim.tick']?.count) throw new Error('No measured tick samples after warm-up in '+capture);
      save();
    }
    if (new Set(continuations).size !== 1) throw new Error(world+' performance captures ended at different continuations');
    summary.checks.push({ name: world+' performance continuation agreement', passed: true, continuation: continuations[0] }); save();
  }
  if (JSON.stringify(sourceState()) !== JSON.stringify(before) || sha(readFileSync(summary.executable)) !== summary.executableSha256) throw new Error('Inputs changed during capture');
  summary.status='passed'; summary.finished_utc=new Date().toISOString(); save();
  console.log('Baseline complete: ' + summaryPath);
} catch (error) {
  summary.status='failed'; summary.failure=error.message; summary.finished_utc=new Date().toISOString(); save(); throw error;
}
