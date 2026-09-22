import fs from 'node:fs';
import path from 'node:path';
import { nativeRecords, framesInWindow, summarizeSamples } from './profile-analysis.mjs';
import { liveSurfaceRates } from './profile-surface-summary.mjs';
import { parse } from 'smol-toml';
const root = path.resolve(process.argv[2]);
const artifact = JSON.parse(fs.readFileSync(path.join(root, 'frames.json')));
const surface = JSON.parse(fs.readFileSync(path.join(root, 'frames.surfaces.json')));
const markers = fs.readFileSync(path.join(root, 'stages.jsonl'), 'utf8').trim().split('\n').map(line => {
  const row = JSON.parse(line); return {...row, ...JSON.parse(decodeURIComponent(row.path.slice(1)))};
});
const records = nativeRecords(artifact);
const diagnosticPath = path.join(root,'diagnostics.json');
const diagnostic = fs.existsSync(diagnosticPath) ? JSON.parse(fs.readFileSync(diagnosticPath)) : null;
const manifest = JSON.parse(fs.readFileSync(path.join(root,'manifest.json')));
const stages = [manifest.stage].map(stage => {
  const start = markers.find(m => m.stage === stage + '-start');
  const end = markers.find(m => m.stage === stage + '-end');
  if (!start || !end) return {stage, error:'No complete measurement window'};
  const from = (start.unixMs - artifact.startedUnixMs) / 1000;
  const seconds = (end.unixMs - start.unixMs) / 1000;
  const stats = summarizeSamples(framesInWindow(records, from, seconds));
  const selected = markers.find(m => m.stage === 'ship');
  const stateVerified = stage === 'idle' ? end.phase === 'Lobby'
    : end.phase === 'InProgress' && (stage === 'running' || (['player','multi'].includes(stage) && end.updates > 0 && end.assigned === 'helm') || (
      end.updates > 0 && end.selectedShip === selected?.ship
      && (stage === 'controlled' ? end.active === 'true' && end.controlLost === false : end.active === 'false')));
  const events = surface.events.filter(e => e.at_ns / 1e9 >= from && e.at_ns / 1e9 < from + seconds);
  const workers = events.filter(e => e.event === 'iteration');
  const phases = Object.fromEntries(['update_ns','pump_ns','render_ns','copy_ns','publish_ns','total_ns'].map(key => [key.replace('_ns', 'Ms'), summarizeSamples(workers.map(e => e[key] / 1e6))]));
  const splitPath=path.join(root,'ultralight.csv');
  const split=fs.existsSync(splitPath) ? fs.readFileSync(splitPath,'utf8').trim().split(/\r?\n/).slice(1).map(row=>row.split(',').map(Number)) : [];
  const allIterations=surface.events.filter(e=>e.event==='iteration');
  const splitAligned=split.length>0 && split.length===allIterations.length;
  if(splitAligned){
    const rows=split.filter((_,i)=>allIterations[i].at_ns/1e9>=from && allIterations[i].at_ns/1e9<from+seconds);
    phases.animationMs=summarizeSamples(rows.map(row=>row[1]/1e6));
    phases.paintMs=summarizeSamples(rows.map(row=>row[2]/1e6));
  }
  const uploaded = events.filter(e => e.event === 'uploaded');
  const surfaces = [...new Set(events.filter(e => e.surface?.visible).map(e => JSON.stringify(e.surface)))].map(row => JSON.parse(row));
  const health = markers.find(m => m.stage === 'health');
  const workload = artifact.workload.filter(w=>w.elapsedSeconds>=from && w.elapsedSeconds<from+seconds);
  const simulation = diagnostic?.workload.filter(w=>w.seconds>=from && w.seconds<from+seconds) || [];
  const invalidPath = path.join(root,'invalid.json');
  const reasons = fs.existsSync(invalidPath) ? JSON.parse(fs.readFileSync(invalidPath)).reasons : [];
  if(fs.existsSync(splitPath) && !splitAligned) reasons.push('split timings do not match iteration count');
  if (!artifact.complete || surface.omitted_events || diagnostic?.truncated) reasons.push('truncated telemetry');
  if (!stateVerified) reasons.push('incorrect end state');
  if (!health || health.invalid.length) reasons.push(...(health?.invalid || ['missing continuous health checks']));
  if (stage === 'multi') {
    const gmHealth=markers.find(m=>m.stage==='gm-health');
    const gmEnd=markers.find(m=>m.stage==='gm-running-end');
    if (!gmHealth || gmHealth.invalid.length || gmEnd?.phase !== 'InProgress') reasons.push('GM surface not verified');
    if (markers.filter(m=>m.stage==='gm-workload').length !== 1) reasons.push('GM surface reload');
    if (workload.some(w=>w.windows.length !== 3)) reasons.push('expected three native windows');
    const expected = manifest.bridgeProfile ? parse(manifest.bridgeProfile).display : [];
    if (expected.length !== 3 || !workload.length || workload.some(w => expected.some(display =>
      !w.windows.some(window => window.monitor === display.id
        && (display.role === 'gm' ? window.title.endsWith('— GM')
          : display.role === 'station' ? window.title.endsWith('— Station')
            : !window.title.endsWith('— GM') && !window.title.endsWith('— Station')))))) {
      reasons.push('three-monitor role placement not verified');
    }
    if (markers.some(m=>m.stage==='gm-error')) reasons.push('GM driver error');
  }
  if (markers.filter(m=>m.stage==='workload').length !== 1) reasons.push('surface reload');
  if (markers.some(m=>m.stage==='error')) reasons.push('driver error');
  if (!simulation.length) reasons.push('missing simulation telemetry');
  if (['observed','player','multi'].includes(stage) && !(health?.changedReadings > 0)) reasons.push('no changing console readings');
  if (stage !== 'idle' && simulation.length > 1) {
    const first = simulation[0], last = simulation.at(-1);
    if (Math.abs((last.virtual_seconds-first.virtual_seconds) / (last.seconds-first.seconds)-1) > 0.03) reasons.push('simulation not real time');
    if (last.tick <= first.tick) reasons.push('no fixed tick advancement');
    if (Math.abs((last.tick-first.tick)/(last.seconds-first.seconds)-60) > 2) reasons.push('fixed rate not maintained');
  }
  if (new Set(workload.map(w=>JSON.stringify(w.windows))).size !== 1) reasons.push('window geometry changed');
  const fps = 1000 / stats.mean;
  const performanceFailures = [];
  const liveSurfaceHz=liveSurfaceRates(events, seconds);
  const liveSurfaceIds=Object.keys(liveSurfaceHz);
  if (fps < 58) performanceFailures.push('outer cadence below approximately 60 FPS');
  if (stats.p50 > 16.7) performanceFailures.push('outer median above 16.7 ms');
  if (stats.p95 >= 25) performanceFailures.push('outer p95 at or above 25 ms');
  if (['observed','player','multi'].includes(stage)) {
    if(!liveSurfaceIds.length || Object.values(liveSurfaceHz).some(hz=>hz<58)) performanceFailures.push('visible native console frames below approximately 60 FPS');
    if ((health?.consoleMs?.count || 0) / seconds < 58) performanceFailures.push('live console cadence below approximately 60 FPS');
    if (!(health?.readingIntervalMs?.p95 < 25)) performanceFailures.push('live console p95 at or above 25 ms');
  }
  return {stage, stateVerified, sampleValid:reasons.length===0, accepted:reasons.length===0 && performanceFailures.length===0, rejectionReasons:reasons, performanceFailures, health, simulation, seconds, startSeconds:from, fps, frameMs:stats, endState:end, worker:phases,
    workerIterationsPerSecond:workers.length / seconds, liveSurfaceHz,
    uploadMBperSecond:uploaded.reduce((sum,e)=>sum+e.pixels*4,0)/1e6/seconds,
    uploadsPerSecond:uploaded.length/seconds, visibleSurfaces:surfaces,
    workload:artifact.workload.filter(w=>w.elapsedSeconds>=from && w.elapsedSeconds<from+seconds).filter((w,i,a)=>!i || JSON.stringify(w.windows)!==JSON.stringify(a[i-1].windows)),
  };
});
const report = {complete:artifact.complete, omittedSurfaceEvents:surface.omitted_events, markers, stages};
fs.writeFileSync(path.join(root,'summary.json'), JSON.stringify(report,null,2));
console.log(JSON.stringify(report,null,2));
