import fs from 'node:fs';
import path from 'node:path';
import { readJson, summarizeSamples, compilerContention, backgroundCpu } from './profile-analysis.mjs';

const root = path.resolve(process.argv[2]);
const runs = fs.readdirSync(root).filter(name => fs.statSync(path.join(root, name)).isDirectory()).map(name => {
  const dir = path.join(root, name);
  const manifest = readJson(path.join(dir, 'manifest.json'));
  const capture = readJson(path.join(dir, 'capture.json'));
  const continuation = readJson(path.join(dir, 'capture.json.continuation.json'));
  const processSamples = readJson(path.join(dir, 'process-samples.json'));
  const samples = capture.series['sim.tick'].samples;
  const reasons = [];
  if (manifest.exitCode !== 0 || manifest.timedOut) reasons.push('Run did not complete');
  if (!manifest.sourceClean || !manifest.buildReceiptVerified || !manifest.provenanceUnchanged || !manifest.isolatedState) reasons.push('Run provenance is unverified');
  if (processSamples.length < 2) reasons.push('Process sampling is incomplete');
  if (compilerContention(processSamples).observed) reasons.push('Concurrent build process observed');
  const background = backgroundCpu(processSamples, [manifest.pid, manifest.harnessPid]);
  if (background === null || background > manifest.maxBackgroundCpuCores) reasons.push('Background CPU exceeded quiet-run allowance');
  if (samples.length <= manifest.excludedUpdates) reasons.push('No samples after warm-up');
  if (capture.series['sim.tick'].unit !== 'millis') reasons.push('Update timing has incompatible units');
  if (!Number.isInteger(continuation.tick) || !/^[0-9a-f]{16}$/i.test(continuation.digest || '')) reasons.push('Missing final continuation');
  return { name, manifest, continuation, reasons, backgroundCpuCores: background,
    update: summarizeSamples(samples.slice(manifest.excludedUpdates)), report: readJson(path.join(dir, 'report.json')) };
});
for (const world of new Set(runs.map(r => r.manifest.world))) {
  const group = runs.filter(r => r.manifest.world === world);
  if (new Set(group.map(r => JSON.stringify(r.continuation))).size !== 1) {
    for (const run of group) run.reasons.push('Repeated final tick/digest mismatch');
  }
}
fs.writeFileSync(path.join(root, 'summary.json'), JSON.stringify(runs, null, 2) + '\n');
console.log(JSON.stringify(runs.map(({ name, update, continuation, reasons }) => ({ name, update, continuation, reasons })), null, 2));
if (runs.some(run => run.reasons.length)) process.exitCode = 2;
