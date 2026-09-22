import fs from 'node:fs';
import path from 'node:path';
import { nativeRecords, framesInWindow, summarizeSamples } from './profile-analysis.mjs';
const root = path.resolve(process.argv[2]);
const artifact = JSON.parse(fs.readFileSync(path.join(root, 'frames.json')));
const surface = JSON.parse(fs.readFileSync(path.join(root, 'frames.surfaces.json')));
const markers = fs.readFileSync(path.join(root, 'stages.jsonl'), 'utf8').trim().split('\n').map(line => {
  const row = JSON.parse(line); return {...row, ...JSON.parse(decodeURIComponent(row.path.slice(1)))};
});
const records = nativeRecords(artifact);
const diagnosticPath = path.join(root,'diagnostics.json');
const diagnostic = fs.existsSync(diagnosticPath) ? JSON.parse(fs.readFileSync(diagnosticPath)) : null;
const stages = [JSON.parse(fs.readFileSync(path.join(root,'manifest.json'))).stage].map(stage => {
  const start = markers.find(m => m.stage === stage + '-start');
  const end = markers.find(m => m.stage === stage + '-end');
  if (!start || !end) return {stage, error:'No complete measurement window'};
  const from = (start.unixMs - artifact.startedUnixMs) / 1000;
  const seconds = (end.unixMs - start.unixMs) / 1000;
  const stats = summarizeSamples(framesInWindow(records, from, seconds));
  const selected = markers.find(m => m.stage === 'ship');
  const stateVerified = stage === 'idle' ? end.phase === 'Lobby'
    : end.phase === 'InProgress' && (stage === 'running' || (
      end.updates > 0 && end.selectedShip === selected?.ship
      && (stage === 'controlled' ? end.active === 'true' && end.controlLost === false : end.active === 'false')));
  const events = surface.events.filter(e => e.at_ns / 1e9 >= from && e.at_ns / 1e9 < from + seconds);
  const workers = events.filter(e => e.event === 'iteration');
  const phases = Object.fromEntries(['update_ns','pump_ns','render_ns','copy_ns','publish_ns','total_ns'].map(key => [key.replace('_ns', 'Ms'), summarizeSamples(workers.map(e => e[key] / 1e6))]));
  const uploaded = events.filter(e => e.event === 'uploaded');
  const surfaces = [...new Set(events.filter(e => e.surface?.visible).map(e => JSON.stringify(e.surface)))].map(row => JSON.parse(row));
  const health = markers.find(m => m.stage === 'health');
  const workload = artifact.workload.filter(w=>w.elapsedSeconds>=from && w.elapsedSeconds<from+seconds);
  const simulation = diagnostic?.workload.filter(w=>w.seconds>=from && w.seconds<from+seconds) || [];
  const invalidPath = path.join(root,'invalid.json');
  const reasons = fs.existsSync(invalidPath) ? JSON.parse(fs.readFileSync(invalidPath)).reasons : [];
  if (!artifact.complete || surface.omitted_events || diagnostic?.truncated) reasons.push('truncated telemetry');
  if (!stateVerified) reasons.push('incorrect end state');
  if (!health || health.invalid.length) reasons.push(...(health?.invalid || ['missing continuous health checks']));
  if (markers.filter(m=>m.stage==='workload').length !== 1) reasons.push('surface reload');
  if (markers.some(m=>m.stage==='error')) reasons.push('driver error');
  if (!simulation.length) reasons.push('missing simulation telemetry');
  if (stage === 'observed' && !(health?.changedReadings > 0)) reasons.push('no changing console readings');
  if (stage !== 'idle' && simulation.length > 1) {
    const first = simulation[0], last = simulation.at(-1);
    if (Math.abs((last.virtual_seconds-first.virtual_seconds) / (last.seconds-first.seconds)-1) > 0.03) reasons.push('simulation not real time');
    if (last.tick <= first.tick) reasons.push('no fixed tick advancement');
    if (Math.abs((last.tick-first.tick)/(last.seconds-first.seconds)-60) > 2) reasons.push('fixed rate not maintained');
  }
  if (new Set(workload.map(w=>JSON.stringify(w.windows))).size !== 1) reasons.push('window geometry changed');
  const fps = 1000 / stats.mean;
  const performanceFailures = [];
  if (fps < 58) performanceFailures.push('outer cadence below approximately 60 FPS');
  if (stats.p50 > 16.7) performanceFailures.push('outer median above 16.7 ms');
  if (stats.p95 >= 25) performanceFailures.push('outer p95 at or above 25 ms');
  if (stage === 'observed') {
    if ((health?.consoleMs?.count || 0) / seconds < 58) performanceFailures.push('live console cadence below approximately 60 FPS');
    if (!(health?.readingIntervalMs?.p95 < 25)) performanceFailures.push('live console p95 at or above 25 ms');
  }
  return {stage, stateVerified, sampleValid:reasons.length===0, accepted:reasons.length===0 && performanceFailures.length===0, rejectionReasons:reasons, performanceFailures, health, simulation, seconds, startSeconds:from, fps, frameMs:stats, endState:end, worker:phases,
    workerIterationsPerSecond:workers.length / seconds,
    uploadMBperSecond:uploaded.reduce((sum,e)=>sum+e.pixels*4,0)/1e6/seconds,
    uploadsPerSecond:uploaded.length/seconds, visibleSurfaces:surfaces,
    workload:artifact.workload.filter(w=>w.elapsedSeconds>=from && w.elapsedSeconds<from+seconds).filter((w,i,a)=>!i || JSON.stringify(w.windows)!==JSON.stringify(a[i-1].windows)),
  };
});
const report = {complete:artifact.complete, omittedSurfaceEvents:surface.omitted_events, markers, stages};
fs.writeFileSync(path.join(root,'summary.json'), JSON.stringify(report,null,2));
console.log(JSON.stringify(report,null,2));
