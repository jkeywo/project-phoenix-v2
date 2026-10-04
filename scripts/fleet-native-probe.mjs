#!/usr/bin/env node
// Bounded native bootstrap evidence, not a six-peer workload acceptance gate.
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import { spawn, spawnSync, execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { fileHash, treeHash } from './profile-provenance.mjs';
import { paneWorkloadScript, instrumentNativeGmModule, nativeObserverReporterScript } from './fleet-native-workload.mjs';

// HTTP containment permits only links inside the served root. Copy and resolve
// input links so the instrumented bundle is independent of the build directory.
export function stageNativeProbeBundle(bundle, destination) {
  // A filter selects Node's JS traversal, which honors dereference on Node 22.
  // Its unfiltered native traversal preserves links (nodejs/node#59168).
  fs.cpSync(bundle, destination, { recursive: true, dereference: true, filter: () => true });
}

export function parseNativeProbeArgs(argv) {
  const allowed = new Set(['binary', 'bundle', 'source', 'out', 'rendezvous', 'origin', 'fleet-code', 'seconds', 'role', 'workload']);
  const values = {};
  for (let i = 0; i < argv.length; i += 2) {
    const name = argv[i]?.replace(/^--/, '');
    if (!argv[i]?.startsWith('--') || !allowed.has(name) || values[name] !== undefined || !argv[i + 1] || argv[i + 1].startsWith('--')) {
      throw new Error(`Unknown, repeated or incomplete option: ${argv[i]}`);
    }
    values[name] = argv[i + 1];
  }
  for (const name of ['binary', 'bundle', 'source', 'out', 'rendezvous', 'origin']) {
    if (!values[name]) throw new Error(`--${name} is required`);
  }
  const seconds = Number(values.seconds ?? 45);
  if (!Number.isSafeInteger(seconds) || seconds < 5 || seconds > 960) throw new Error('--seconds must be an integer from 5 to 960');
  for (const name of ['rendezvous', 'origin']) {
    const url = new URL(values[name]);
    if (!['http:', 'https:', 'ws:', 'wss:'].includes(url.protocol)) throw new Error(`Invalid --${name} URL`);
  }
  const role = values.role ?? 'ship';
  if (!['ship', 'gm'].includes(role) || (role === 'gm' && !values['fleet-code'])) throw new Error('--role gm requires --fleet-code');
  if (values.workload !== undefined && !['true','false'].includes(values.workload)) throw new Error('--workload requires true or false');
  return { ...values, role, seconds, workload: values.workload === 'true' };
}

// Wrap only the exported adapter's callbacks. The shipped transport, admission,
// bridge queues, simulation and renderer remain the code under observation.
export function instrumentNativeFleetModule(source, endpoint, { gmJoinCode = null, claim = null, recoveryControl = false, deferJoin = false } = {}) {
  const declaration = 'export function createNativeFleetPeer(';
  if (source.split(declaration).length !== 2) throw new Error('Native fleet factory declaration changed');
  return source.replace(declaration, 'function observedNativeFleetPeer(') + `
export function createNativeFleetPeer(options = {}) {
  const endpoint = ${JSON.stringify(endpoint)};
  ${nativeObserverReporterScript(endpoint)}
  report('engine', {userAgent:navigator.userAgent,queueMicrotask:typeof queueMicrotask});
  window.addEventListener('error', event => report('page-error', {message:event.message,stack:event.error?.stack}));
  window.addEventListener('unhandledrejection', event => report('page-rejection', {message:String(event.reason)}));
  let lastHealth = 0;
  let armedDivergence = false, injected = false;
  let recoverySocket=null, memberCreations=0, wireGeneration=null, lastIdentity=null, redialIdentity=null;
  const observedMember = memberOptions => {
    memberCreations++;
    const socket = memberOptions.factories.socket;
    return (options.createMember || createFleetMember)({...memberOptions,factories:{...memberOptions.factories,
      socket:(...args)=>{ recoverySocket=socket(...args); return recoverySocket; }}});
  };
  const wrap = name => value => {
    if (name !== 'onHealth' || Date.now() - lastHealth > 1000) { report(name,value); if (name === 'onHealth') lastHealth = Date.now(); }
    return options[name]?.(value);
  };
  const peer = observedNativeFleetPeer({...options,
    onRoster:wrap('onRoster'), onDiag:wrap('onDiag'), onHealth:wrap('onHealth'),
    createMember:observedMember,
    send(record) {
      if (['fleet_wire_send','fleet_wire_open'].includes(record.kind)) wireGeneration=record.generation ?? 0;
      if (['fleet_wire_open','fleet_wire_close'].includes(record.kind))
        report(record.kind,{generation:record.generation,role:record.role});
      if(record.kind==='fleet_identity') {
        const identity=record.identity;
        if(redialIdentity) report('redial-identity',{
          sameCredential:!!identity.reconnectCredential && identity.reconnectCredential===redialIdentity.reconnectCredential,
          sameOperator:identity.operatorId===redialIdentity.operatorId,
          sameSlot:identity.claim===redialIdentity.claim});
        lastIdentity=identity;
      }
      if (armedDivergence && !injected && record.kind === 'fleet_frame') {
        const frame = typeof record.frame === 'string' ? JSON.parse(record.frame) : record.frame;
        const command = frame.t === 'tick' && frame.d.commands?.find(row => row.payload?.type === 'SetBoost');
        if (command) {
          const original = command.payload.data.active;
          command.payload.data.active = !original; injected = true;
          report('divergence-injected',{from:frame.d.from,authenticatedSlot:record.authenticated_slot,
            origin:command.origin,seq:command.seq,tick:command.tick,ship:command.ship,original,changed:!original});
          record = {...record,frame:typeof record.frame === 'string' ? JSON.stringify(frame) : frame};
        }
      }
      // Never write reconnect capabilities or the unrestricted wire payload.
      if (record.kind === 'fleet_roster') {
        try {
          const roster = JSON.parse(record.roster);
          report('simulation-roster',{generation:record.generation,local:roster.local,participants:roster.participants,
            shipHosts:(roster.ships || []).map(ship=>ship.host),gmHosts:(roster.gms || []).map(gm=>gm.host)});
        } catch (_) { report('observer-error',{reason:'invalid-simulation-roster'}); }
      }
      if (['fleet_code','fleet_join_status','fleet_fault','fleet_slot_claimed'].includes(record.kind)) report(record.kind, record);
      return options.send(record);
    },
  });
  const receive = peer.receive;
  peer.receive = raw => {
    try {
      const event=JSON.parse(raw)?.native_wire;
      if(event && ['open','close','fault'].includes(event.event))
        report('fleet-wire-event',{generation:event.generation,event:event.event});
    } catch (_) {}
    return receive.call(peer,raw);
  };
  const configure = peer.configure;
  peer.configure = raw => {
    const config = typeof raw === 'string' ? JSON.parse(raw) : raw;
    report('configuration', {owner:config.owner, base:config.base, stamp:config.stamp, ship_path:config.ship_path});
    try { return configure.call(peer, raw); } catch (error) { report('configure-error', {message:error.message,stack:error.stack}); throw error; }
  };
  const update = peer.update;
  let last = '';
  peer.update = raw => {
    const state = typeof raw === 'string' ? JSON.parse(raw) : raw;
    for (const rawFrame of state.frames || []) {
      try {
        const frame = JSON.parse(rawFrame);
        if (frame.t === 'digest') report('digest',frame.d);
        if (frame.t === 'tick') for (const command of frame.d.commands || []) report('recovery-command', {
          from:frame.d.from, origin:command.origin, seq:command.seq, tick:command.tick,
          ship:command.ship, target:command.target, type:command.payload?.type
        });
        if (['host-loss','slot-claim','recovery-ready'].includes(frame.t)) report('recovery-frame',frame);
      } catch (_) { report('observer-error',{reason:'invalid-outgoing-frame'}); }
    }
    const evidence = {crew:state.crew, ship_ready:state.ship_ready, gm_ready:state.gm_ready, validation:state.validation, roster_result:state.roster_result, recovery:state.recovery, gm_join:state.gm_join, continuation:state.continuation_result?.status};
    const key = JSON.stringify(evidence);
    if (key !== last) { last = key; report('state', evidence); }
    return update.call(peer, raw);
  };
  const join = peer.join;
  const replacementClaim = ${JSON.stringify(claim)};
  let deferredArgs = null;
  const executeJoin = (args,attempt=null) => {
    if (replacementClaim) args[2] = {...args[2],claim:replacementClaim};
    const startedMs=Date.now(),result=join.apply(peer,args);
    report('join-call',{role:args[3],accepted:result===true,startedMs,attempt}); return result;
  };
  peer.join = (...args) => {
    if (${JSON.stringify(deferJoin)}) { deferredArgs=args; report('join-deferred',{role:args[3]}); return true; }
    return executeJoin(args);
  };
  ${recoveryControl ? `let controlBusy=false;
  setInterval(async()=>{
    if(controlBusy)return; controlBusy=true;
    try {
      const response=await fetch(endpoint+'/fleet-control');
      if(!response.ok)throw new Error('Fleet control HTTP '+response.status);
      const command=await response.json();
      if(command?.kind==='join') {
        if(!deferredArgs)throw new Error('No deferred native join');
        const args=[...deferredArgs];
        setTimeout(()=>executeJoin(args,command.attempt),Math.max(0,command.startAt-Date.now()));
      }
      if(command?.kind==='diverge') {armedDivergence=true; report('divergence-armed',{});}
      if(command?.kind==='gm-redial') {
        if(redialIdentity || peer.role!=='gm' || !lastIdentity?.reconnectCredential
          || recoverySocket?.readyState!==1 || !Number.isSafeInteger(wireGeneration))
          throw new Error('Native GM socket redial precondition missing');
        redialIdentity={...lastIdentity};
        report('redial-requested',{generation:wireGeneration,memberCreations});
        recoverySocket.close();
      }
    } catch(error){report('observer-error',{reason:error.message});}
    finally {controlBusy=false;}
  },100);` : ''}
  const gmJoinCode = ${JSON.stringify(gmJoinCode)};
  if (gmJoinCode) setTimeout(() => {
    window.phoenixHostLobbyOut.send(JSON.stringify({kind:'join_peer',code:gmJoinCode}));
    window.phoenixHostLobbyOut.send(JSON.stringify({kind:'select_scenario',scenario_id:'matrix_probe'}));
    report('join-requested', {role:'gm'});
  }, 1000);
  return peer;
}
`;
}

export function nativeProbeOutcome(events, processExit, expected = 'owner') {
  const engineLoaded = events.some(event => event.kind === 'engine');
  const configured = events.some(event => event.kind === 'configuration');
  const registered = events.some(event => event.kind === 'fleet_code');
  const admitted = events.some(event => event.kind === 'fleet_join_status' && event.value?.status === 'admitted');
  const routes = events.filter(event => event.kind === 'onDiag' && event.value?.event === 'transport').map(event => event.value);
  return { engineLoaded, configured, registered, admitted, observedRoutes: routes,
    bootstrapObserved: engineLoaded && configured && (expected === 'member' ? admitted : registered) && processExit === null,
    sixPeerWorkloadPassed: false,
    limits: ['Single native bootstrap only; no twelve-client workload or recorded GM actions',
      'No launch, simulation agreement, impairment, endurance or physical network claim',
      'Configured relay capability is not an observed link route; route callbacks are recorded separately'] };
}

// The observer owns only this spawned child. Waiting is bounded, and a forced
// Windows cleanup targets its exact process tree rather than a process name.
export async function terminateNativeProbe(child, { graceMs = 5000, forceMs = 1000,
  forceTree = pid => {
    const result = spawnSync('taskkill', ['/PID',String(pid),'/T','/F'], {windowsHide:true,timeout:5000,encoding:'utf8'});
    return {status:result.status,error:result.error?.message};
  } } = {}) {
  const exited = () => child.exitCode !== null || child.signalCode !== null;
  if (exited()) return {cleanupExitObserved:true};
  if (!Number.isInteger(child.pid) || child.pid <= 0) throw new Error('Native cleanup requires the spawned child PID');
  const waitExit = ms => new Promise(resolve => {
    if (exited()) { resolve(); return; }
    const finish = () => { clearTimeout(timer); child.removeListener('exit',finish); resolve(); };
    const timer = setTimeout(finish,ms);
    child.once('exit',finish);
  });
  const result = {};
  const normal = waitExit(graceMs);
  try { child.kill(); } catch (error) { result.killError = error.message; }
  await normal;
  if (!exited()) {
    try { result.forcedCleanup = forceTree(child.pid); }
    catch (error) { result.forcedCleanup = {error:error.message}; }
    await waitExit(forceMs);
  }
  result.cleanupExitObserved = exited();
  return result;
}

export async function runNativeProbe(options) {
  if (!Number.isSafeInteger(options.seconds) || options.seconds < 5 || options.seconds > 960) throw new Error('Native lifetime must be an integer from 5 to 960 seconds');
  if (process.platform !== 'win32') throw new Error('This probe requires Windows');
  const source = fs.realpathSync(options.source);
  const binary = fs.realpathSync(options.binary);
  const bundle = fs.realpathSync(options.bundle);
  const output = path.resolve(options.out);
  if (fs.existsSync(output)) throw new Error('Output directory already exists');
  if (output === bundle || output.startsWith(bundle + path.sep)) throw new Error('Output must be outside the input bundle');
  // Fail before starting any process if the real input documents are absent.
  const modulePath = path.join(bundle, 'gui/native-fleet-peer.js');
  const fleetSource = fs.readFileSync(modulePath, 'utf8');
  fs.accessSync(path.join(bundle, 'index.html'));
  const revision = execFileSync('git', ['-C', source, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
  const dirty = execFileSync('git', ['-C', source, 'status', '--porcelain'], { encoding: 'utf8' });
  fs.mkdirSync(output, { recursive: true });
  const harnessRoot = path.dirname(fileURLToPath(import.meta.url));
  fs.mkdirSync(path.join(output,'harness'));
  const harnessHashes = {};
  for (const name of ['fleet-native-probe.mjs','fleet-native-matrix.mjs','fleet-native-workload.mjs',
    'fleet-native-recovery.mjs','fleet-mixed-recovery.mjs','fleet-effect-witness.mjs','fleet-browser-recovery.mjs']) {
    const file = path.join(harnessRoot,name);
    fs.copyFileSync(file,path.join(output,'harness',name));
    harnessHashes[name] = fileHash(file);
  }
  const events = [];
  let eventOverflow = false;
  const eventPath = `/${randomUUID()}`;
  const observerFault = reason => {
    if (eventOverflow) return;
    eventOverflow = true;
    const event = {at:new Date().toISOString(),kind:'observer-overflow',value:{reason}};
    events.push(event); options.onEvent?.(event);
    fs.appendFileSync(path.join(output,'events.ndjson'),JSON.stringify(event) + '\n');
  };
  const listener = http.createServer({maxHeaderSize:131072}, (req, res) => {
    const url = new URL(req.url, 'http://127.0.0.1');
    if (req.method === 'GET' && url.pathname === eventPath + '/control') {
      res.writeHead(200, {'Content-Type':'application/json','Access-Control-Allow-Origin':'*'}).end(JSON.stringify(options.nextCommand?.() || null)); return;
    }
    if (req.method === 'GET' && url.pathname === eventPath + '/fleet-control') {
      res.writeHead(200, {'Content-Type':'application/json','Access-Control-Allow-Origin':'*'}).end(JSON.stringify(options.nextFleetCommand?.() || null)); return;
    }
    if (req.method !== 'GET' || url.pathname !== eventPath) { res.writeHead(404).end(); return; }
    if ((req.url?.length || 0) > 65536 || events.length >= 20000) {
      observerFault('event-bound');
      res.writeHead(413).end(); return;
    }
    try {
      const event = JSON.parse(url.searchParams.get('event'));
      if (!event || typeof event.kind !== 'string') throw new Error('Invalid event');
      const observed = { at: new Date().toISOString(), ...event };
      events.push(observed);
      fs.appendFileSync(path.join(output, 'events.ndjson'), JSON.stringify(observed) + '\n');
      options.onEvent?.(observed);
      res.writeHead(204, { 'Access-Control-Allow-Origin': '*' }).end();
    } catch { res.writeHead(400).end(); }
  });
  listener.on('clientError', (error, socket) => {
    observerFault('http-parser-' + error.code);
    socket.end('HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n');
  });
  const evidence = { kind: 'native-runtime-bootstrap-only', sourceRevision: revision, dirtySource: !!dirty.trim(),
    binary: { path: binary, sha256: fileHash(binary), sourceBuildVerified: false },
    bundle: { path: bundle, indexSha256: fileHash(path.join(bundle, 'index.html')), guiSha256: treeHash(path.join(bundle, 'gui')) },
    sdk: Object.fromEntries(['Ultralight.dll', 'UltralightCore.dll', 'WebCore.dll', 'AppCore.dll'].map(name => {
      const file = path.join(path.dirname(binary), name); return [name, fs.existsSync(file) ? fileHash(file) : null];
    })),
    hardware: { os: os.version(), release: os.release(), cpu: os.cpus()[0]?.model, logicalCpus: os.cpus().length, ramBytes: os.totalmem() },
    identityStoreRoot:path.join(output,'fleet-identities'), harnessHashes, node: process.version, bounds: { seconds: options.seconds, maxEvents: 20000 },
    routeIntent: 'ws-relay', processExit: null, failure: null };
  let child;
  let stdout;
  let stderr;
  let spawnError;
  try {
    await new Promise((resolve, reject) => { listener.once('error', reject); listener.listen(0, '127.0.0.1', resolve); });
    const endpoint = `http://127.0.0.1:${listener.address().port}${eventPath}`;
    const privateContent = path.join(output, 'content');
    fs.mkdirSync(privateContent);
    fs.writeFileSync(path.join(privateContent,'matrix-scenarios.toml'),'[content]\nid = "phoenix-base"\nepoch = 1\n[[scenario]]\nid = "matrix_probe"\nworld = "assets/worlds/probe_fleet_six_peer.toml"\n');
    fs.symlinkSync(path.join(source, 'assets'), path.join(privateContent, 'assets'), 'junction');
    const scratchBundle = path.join(output, 'bundle');
    stageNativeProbeBundle(bundle, scratchBundle);
    const instrumented = instrumentNativeFleetModule(fleetSource, endpoint, {gmJoinCode: options.role === 'gm' ? options['fleet-code'] : null,
      claim: options.claim || null,recoveryControl:!!options.nextFleetCommand,deferJoin:options.deferJoin===true});
    fs.writeFileSync(path.join(scratchBundle, 'gui/native-fleet-peer.js'), instrumented);
    if (options.workload) {
      const gmPath = path.join(scratchBundle, 'gui/native-gm-workspace.js');
      fs.writeFileSync(gmPath, instrumentNativeGmModule(fs.readFileSync(gmPath,'utf8'), endpoint, {deferReady:options.deferGmReady === true}));
      const client = path.join(scratchBundle,'client');
      const index = path.join(client,'index.html');
      fs.writeFileSync(index, fs.readFileSync(index,'utf8').replace('</body>', '<script>' + paneWorkloadScript(endpoint) + '</script></body>'));
    }
    evidence.bundle.instrumentedModuleSha256 = fileHash(path.join(scratchBundle, 'gui/native-fleet-peer.js'));
    evidence.bundle.originalModuleSha256 = fileHash(modulePath);
    const args = [...(options.role === 'gm' ? ['--lobby'] : ['--world', 'assets/worlds/probe_fleet_six_peer.toml', '--ship', 'assets/entities/alliance_cruiser.toml', '--seed', '1519']), '--content-dir', privateContent, '--manifest', 'matrix-scenarios.toml', '--client-dir', scratchBundle, '--addr', '127.0.0.1:0',
      '--rendezvous', options.rendezvous, '--origin', options.origin,
      '--save-dir', path.join(output, 'saves'), '--log', 'info'];
    if (options.role !== 'gm' && options['fleet-code']) args.push('--fleet-code', options['fleet-code']);
    if (options.workload && options.role !== 'gm') for (const station of ['captain','helm','engineering']) args.push('--pane','matrix-' + station);
    evidence.arguments = args;
    evidence.startedAt = new Date().toISOString();
    stdout = fs.openSync(path.join(output, 'stdout.log'), 'wx');
    stderr = fs.openSync(path.join(output, 'stderr.log'), 'wx');
    child = spawn(binary, args, { cwd: output, windowsHide: true,
      env: { ...process.env, BEVY_ASSET_ROOT: privateContent, PHOENIX_FLEET_IDENTITY_DIR:evidence.identityStoreRoot, APPDATA: path.join(output, 'appdata'), LOCALAPPDATA: path.join(output, 'localappdata'), RUST_LOG: 'warn,bevy_render::renderer=info' },
      stdio: ['ignore', stdout, stderr] });
    child.once('error', error => { spawnError = error; });
    evidence.processId=child.pid;
    options.onProcess?.(()=>({pid:child.pid,startedAt:evidence.startedAt,alive:child.exitCode===null&&child.signalCode===null&&!spawnError}));
    const deadline = Date.now() + options.seconds * 1000;
    while (Date.now() < deadline && child.exitCode === null && child.signalCode === null && !spawnError && !eventOverflow && !options.shouldStop?.()) await new Promise(resolve => setTimeout(resolve, 100));
    if (spawnError) throw spawnError;
    if (eventOverflow) throw new Error('Native observer event bound exceeded');
    evidence.processExit = child.exitCode ?? child.signalCode;
    evidence.outcome = nativeProbeOutcome(events, evidence.processExit, options['fleet-code'] ? 'member' : 'owner');
  } catch (error) { evidence.failure = error.message; evidence.outcome = nativeProbeOutcome(events, child?.exitCode ?? -1, options['fleet-code'] ? 'member' : 'owner'); }
  finally {
    if (child && child.exitCode === null && child.signalCode === null && !spawnError) {
      Object.assign(evidence, await terminateNativeProbe(child));
      evidence.terminatedAtBound = true;
      if (!evidence.cleanupExitObserved) evidence.failure ||= 'Native process cleanup did not confirm exit';
    }
    listener.closeAllConnections();
    if (listener.listening) await new Promise(resolve => { const timer=setTimeout(()=>{evidence.failure ||= 'Observer listener cleanup timed out';resolve();},3000); listener.close(()=>{clearTimeout(timer);resolve();}); });
    if (stdout !== undefined) fs.closeSync(stdout);
    if (stderr !== undefined) fs.closeSync(stderr);
    evidence.finishedAt = new Date().toISOString();
    evidence.nativeEngineUserAgent = events.find(event => event.kind === 'engine')?.value.userAgent || null;
    const stderrPath = path.join(output,'stderr.log');
    evidence.hardware.gpuAdapterLog = fs.existsSync(stderrPath) ? fs.readFileSync(stderrPath,'utf8').split(/\r?\n/).find(line => line.includes('AdapterInfo')) || null : null;
    evidence.binaryUnchanged = fileHash(binary) === evidence.binary.sha256;
    fs.writeFileSync(path.join(output, 'events.json'), JSON.stringify(events, null, 2));
    fs.writeFileSync(path.join(output, 'source.patch'), execFileSync('git', ['-C', source, 'diff', 'HEAD'], { encoding: 'utf8' }));
    fs.writeFileSync(path.join(output, 'manifest.json'), JSON.stringify(evidence, null, 2));
  }
  return evidence;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const result = await runNativeProbe(parseNativeProbeArgs(process.argv.slice(2)));
  console.log(JSON.stringify({ output: path.resolve(process.argv[process.argv.indexOf('--out') + 1]), ...result.outcome, failure: result.failure }, null, 2));
  if (!result.outcome.bootstrapObserved || result.failure) process.exitCode = 1;
}
