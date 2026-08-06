// Dev launcher for the model viewer (npm run dev:viewer).
//
// Starts two things:
//   1. A tiny server on ASSET_PORT that serves ./assets and the /api/ routes
//      the viewer's LOD panel calls.
//   2. `trunk serve` on 8081, which proxies both paths to (1) — see the
//      [[proxy]] blocks in viewer-trunk.toml.
//
// Why not Trunk's own `rel="copy-dir"`, as server.html uses? The assets tree is
// ~300 MB, and Trunk's staging rename of it fails on Windows with "Access is
// denied" (os error 5) whenever the output directory is fresh. Proxying avoids
// the copy entirely, which also means an edited .wgsl or .glb is live on the
// next page load rather than after a 300 MB restage.
//
// ── The /api/ half ──────────────────────────────────────────────────────────
//
// The viewer can author a model's LOD ladder and run the generator over it.
// That is filesystem work, and the page is wasm in a browser, so it goes
// through this process. Everything here is the *edge*: path resolution, reads,
// writes and one child process. The rules about what a ladder may say live in
// scripts/viewer-lods.mjs (pure, unit-tested) and, for the parameters, in
// scripts/generate-lods.mjs — this file does not get its own opinion about
// them.
//
// It binds localhost only and refuses any path outside assets/models. It is a
// dev tool that writes to the working tree by design: nothing here should ever
// be reachable from a deployed page.

import { createServer } from 'node:http';
import { createReadStream } from 'node:fs';
import { stat, readdir, readFile, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import path from 'node:path';
import { parse as parseToml } from 'smol-toml';

import {
  modelStem,
  sidecarsForStem,
  variantOfSidecar,
  ladderFromDoc,
  extentFromDoc,
  modelUnitExtent,
  templateLadder,
  replaceLadder,
  validateLadder,
  validateProposal,
  generatedOutputs,
} from './viewer-lods.mjs';

const ASSET_PORT = 8082;
const ROOT = process.cwd();
const ASSETS_ROOT = path.join(ROOT, 'assets');
const MODELS_DIR = path.join(ASSETS_ROOT, 'models');
const GENERATOR = path.join(ROOT, 'scripts', 'generate-lods.mjs');

const MIME = {
  '.glb': 'model/gltf-binary',
  '.gltf': 'model/gltf+json',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
  '.ktx2': 'image/ktx2',
  '.toml': 'text/plain; charset=utf-8',
  '.json': 'application/json',
  '.wgsl': 'text/plain; charset=utf-8',
  '.ogg': 'audio/ogg',
  '.mp3': 'audio/mpeg',
  '.ttf': 'font/ttf',
};

// ── The generator run slot ──────────────────────────────────────────────────
//
// One at a time, process-wide. Two concurrent runs over the same model would
// race on the same .glb and on scripts/lod-manifest.toml, and the manifest is
// the thing CI trusts to say the tree is current — a half-written one is worse
// than no run at all. The panel polls this rather than waiting on the POST: a
// stubborn mesh takes minutes, and a page that cannot show the log until the
// end is a page that looks hung.
const run = {
  active: false,
  model: null,
  command: null,
  log: '',
  code: null,
  startedAt: null,
};

const json = (res, status, body) => {
  const text = JSON.stringify(body);
  res.writeHead(status, {
    'Content-Type': 'application/json; charset=utf-8',
    'Content-Length': Buffer.byteLength(text),
    'Cache-Control': 'no-store',
  });
  res.end(text);
};

const readBody = (req) =>
  new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    req.on('data', (chunk) => {
      size += chunk.length;
      // A ladder is a few hundred bytes; anything near a megabyte is a bug or
      // a mistake, and buffering it would be this process's problem either way.
      if (size > 1_000_000) reject(new Error('request body too large'));
      else chunks.push(chunk);
    });
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
    req.on('error', reject);
  });

/**
 * Resolve a `assets/models/<name>.glb` request path to its stem, or throw.
 *
 * Everything the API touches is derived from this stem, so this is the one
 * place that has to be strict: a name is a bare file stem with no separators,
 * and it has to name a `.glb` that exists.
 */
async function resolveModel(modelPath) {
  if (typeof modelPath !== 'string' || !modelPath) throw new Error('no model given');
  const stem = modelStem(modelPath);
  if (!/^[A-Za-z0-9_-]+$/.test(stem)) throw new Error(`not a model name: ${modelPath}`);
  const glb = path.join(MODELS_DIR, `${stem}.glb`);
  await stat(glb).catch(() => {
    throw new Error(`no such model: assets/models/${stem}.glb`);
  });
  return stem;
}

/** Byte size of a repo-relative path, or `null` when it is not there yet. */
async function bytesOf(relative) {
  return stat(path.join(ROOT, relative))
    .then((s) => s.size)
    .catch(() => null);
}

/**
 * Everything the panel needs about one model's ladder: which sidecars carry it,
 * what it says, and how big each level's file currently is.
 *
 * **The ladder reported is the requested variant's, and only that one.** The
 * renderer resolves exactly one sidecar — `<stem>.<variant>.toml` — and an
 * absent one means an identity rig with no ladder at all. Falling back to
 * "whichever sidecar has a ladder" is what this used to do, and it made the
 * panel disagree with the screen: the asteroids ship no `<stem>.model.toml`, so
 * the base variant offered four levels to click on that the engine could not
 * see. `ladderVariants` names the variants that do carry one, so the panel can
 * move to a variant that renders instead of showing a ladder that will not.
 *
 * The variants are required to agree about the levels (they share the generated
 * files), but *not* about the extents — each applies its own base scale, and
 * the extents are what size a procedural level and scale a copied ladder.
 */
async function ladderState(stem, variant) {
  const files = await readdir(MODELS_DIR);
  const sidecars = sidecarsForStem(files, stem);
  const wanted = `${stem}.${variant || 'model'}.toml`;

  let levels = [];
  let extent = null;
  let modelExtent = null;
  const carriers = [];
  for (const file of sidecars) {
    const doc = parseToml(await readFile(path.join(MODELS_DIR, file), 'utf8'));
    const declared = ladderFromDoc(doc);
    carriers.push({
      path: `assets/models/${file}`,
      variant: variantOfSidecar(file, stem),
      levels: declared.length,
    });
    if (file === wanted) {
      levels = declared;
      extent = extentFromDoc(doc);
      modelExtent = modelUnitExtent(doc);
    }
  }
  // No sidecar for this variant: the model still has extents to frame and scale
  // by, and any sidecar's are better than none.
  if (extent === null) {
    for (const file of sidecars) {
      const doc = parseToml(await readFile(path.join(MODELS_DIR, file), 'utf8'));
      extent = extentFromDoc(doc);
      modelExtent = modelUnitExtent(doc);
      if (extent !== null) break;
    }
  }

  const sized = [];
  for (const level of levels) {
    sized.push({
      model: level.model ?? null,
      bytes: level.model ? await bytesOf(level.model) : null,
    });
  }

  return {
    stem,
    variant: variant || '',
    // The variants whose sidecar carries a ladder, so a panel sitting on one
    // that does not can move to one that does.
    ladderVariants: carriers.filter((c) => c.levels).map((c) => c.variant),
    extent,
    // The model's own units, which is what `remesh_voxel_size` is measured in.
    modelExtent,
    source: {
      model: `assets/models/${stem}.glb`,
      bytes: await bytesOf(`assets/models/${stem}.glb`),
    },
    sidecars: carriers,
    levels,
    files: sized,
    outputs: generatedOutputs(levels),
  };
}

/**
 * A model's ladder as authored, whichever variant declares it.
 *
 * For questions about the ladder's *shape* rather than about what is on screen
 * — copying it as a template, or asking whether there is anything to generate.
 * [`ladderState`] is deliberately variant-scoped because the renderer is; these
 * questions are not.
 */
async function anyLadderState(stem) {
  const state = await ladderState(stem, '');
  if (state.levels.length || !state.ladderVariants.length) return state;
  return ladderState(stem, state.ladderVariants[0]);
}

/**
 * Every generated `.glb` any sidecar of this model declares.
 *
 * This is the precondition for a run, and it has to be collected the way
 * `generate-lods.mjs` collects it — across *all* the model's sidecars. Asking
 * one variant is what made the panel refuse to run over an asteroid ("declares
 * no [lod.generate] level") whose ladder lives in the small/large/huge/cosmetic
 * sidecars and not in a base one it has never had.
 */
async function generatedOutputsForStem(stem) {
  const files = await readdir(MODELS_DIR);
  const outputs = new Set();
  for (const file of sidecarsForStem(files, stem)) {
    const doc = parseToml(await readFile(path.join(MODELS_DIR, file), 'utf8'));
    for (const output of generatedOutputs(ladderFromDoc(doc))) outputs.add(output);
  }
  return [...outputs];
}

/**
 * Level count and extent for every model.
 *
 * The panel uses it twice: to mark which models still have no ladder (the whole
 * authoring backlog, visible in the dropdown), and to offer the ones that do as
 * templates for the ones that do not.
 */
async function ladderIndex() {
  const files = await readdir(MODELS_DIR);
  const stems = files.filter((f) => f.endsWith('.glb')).map((f) => f.slice(0, -'.glb'.length));
  const out = {};
  for (const stem of stems) {
    let levels = 0;
    let generated = 0;
    let extent = null;
    for (const file of sidecarsForStem(files, stem)) {
      const doc = parseToml(await readFile(path.join(MODELS_DIR, file), 'utf8'));
      const declared = ladderFromDoc(doc);
      levels = Math.max(levels, declared.length);
      generated = Math.max(generated, generatedOutputs(declared).length);
      extent = extent ?? extentFromDoc(doc);
    }
    out[stem] = { levels, generated, extent };
  }
  return out;
}

/**
 * Write a ladder into every sidecar of `stem`.
 *
 * Validated twice on purpose, and neither check is this file's own: the shape
 * rules come from viewer-lods.mjs and the parameter rules from the generator's
 * `collectTargets`, run over the *proposed* text of every sidecar. Nothing is
 * written unless both pass, so a rejected save leaves the tree exactly as it
 * was rather than half-updated across variants.
 */
async function saveLadder(stem, levels) {
  const problems = validateLadder(levels);
  if (problems.length) return { ok: false, problems };

  const files = await readdir(MODELS_DIR);
  const sidecars = sidecarsForStem(files, stem);
  if (!sidecars.length) {
    return { ok: false, problems: [`${stem} has no rig sidecar to write the ladder into`] };
  }

  const proposed = [];
  for (const file of sidecars) {
    const current = await readFile(path.join(MODELS_DIR, file), 'utf8');
    proposed.push({ path: `assets/models/${file}`, text: replaceLadder(current, levels), current });
  }

  const declared = validateProposal(proposed.map(({ path: p, text }) => ({ path: p, text })));
  if (declared.length) return { ok: false, problems: declared };

  const written = [];
  for (const item of proposed) {
    if (item.text === item.current) continue; // unchanged: leave the mtime alone
    await writeFile(path.join(ROOT, item.path), item.text);
    written.push(item.path);
  }
  return { ok: true, written, sidecars: proposed.map((p) => p.path) };
}

/**
 * Start a generator run. `filter` is what the generator matches targets on — a
 * model stem for the whole ladder, or one level's output path for that level
 * alone. Returns false when a run is already in flight.
 */
function startRun(filter, { remesh = false, force = false } = {}) {
  if (run.active) return false;
  const args = [GENERATOR, filter];
  if (remesh) args.push('--remesh');
  if (force) args.push('--force');

  run.active = true;
  run.model = filter;
  run.command = `node scripts/generate-lods.mjs ${args.slice(1).join(' ')}`;
  run.log = `$ ${run.command}\n`;
  run.code = null;
  run.startedAt = Date.now();

  const child = spawn(process.execPath, args, { cwd: ROOT });
  // The generator writes its progress to stderr and its plan to stdout; the
  // panel shows one transcript, so they are interleaved here as the terminal
  // would interleave them.
  const append = (chunk) => {
    run.log += chunk.toString();
  };
  child.stdout.on('data', append);
  child.stderr.on('data', append);
  child.on('error', (err) => {
    run.log += `\n[dev-viewer] could not start the generator: ${err.message}\n`;
    run.code = -1;
    run.active = false;
  });
  child.on('close', (code) => {
    run.log += `\n[dev-viewer] exit ${code}\n`;
    run.code = code ?? -1;
    run.active = false;
  });
  return true;
}

// ── Routing ─────────────────────────────────────────────────────────────────

/** Validate a rig variant name: a bare sidecar name part, or empty for base. */
function checkVariant(variant) {
  if (variant && !/^[A-Za-z0-9_-]+$/.test(variant)) throw new Error(`not a variant: ${variant}`);
  return variant || '';
}

/**
 * The rig variant a request names.
 *
 * Query string for the GETs, and the JSON body for a POST that has one — a POST
 * carrying its variant in the body while this only read the query is how the
 * per-level generate route ended up resolving the base variant of an asteroid
 * that has no base sidecar, and refusing every level as "not generated".
 */
function variantOf(url, body) {
  return checkVariant(body?.variant ?? url.searchParams.get('variant') ?? '');
}

async function handleApi(req, res, url) {
  const route = url.pathname.replace(/^\/api\/?/, '');

  if (req.method === 'GET' && route === 'lod') {
    const stem = await resolveModel(url.searchParams.get('model'));
    return json(res, 200, await ladderState(stem, variantOf(url)));
  }

  if (req.method === 'GET' && route === 'lod/index') {
    return json(res, 200, { levels: await ladderIndex() });
  }

  if (req.method === 'GET' && route === 'lod/template') {
    const from = await resolveModel(url.searchParams.get('from'));
    const to = await resolveModel(url.searchParams.get('model'));
    const source = await anyLadderState(from);
    const target = await ladderState(to, variantOf(url));
    if (!source.levels.length) {
      return json(res, 400, { problems: [`${from} has no ladder to copy`] });
    }
    return json(res, 200, {
      levels: templateLadder(
        { levels: source.levels, stem: from, extent: source.extent },
        { stem: to, extent: target.extent },
      ),
      from,
      scaledBy: source.extent && target.extent ? target.extent / source.extent : 1,
    });
  }

  if (req.method === 'POST' && route === 'lod') {
    const body = JSON.parse(await readBody(req));
    const stem = await resolveModel(body.model);
    const levels = Array.isArray(body.levels) ? body.levels : [];
    const result = await saveLadder(stem, levels);
    if (!result.ok) return json(res, 400, result);
    return json(res, 200, { ...result, state: await ladderState(stem, variantOf(url, body)) });
  }

  if (req.method === 'POST' && route === 'lod/generate') {
    const body = JSON.parse(await readBody(req));
    const stem = await resolveModel(body.model);
    const outputs = await generatedOutputsForStem(stem);
    if (!outputs.length) {
      return json(res, 400, {
        problems: [
          `${stem} declares no [lod.generate] level — there is nothing for the generator to make`,
        ],
      });
    }

    // One level, when the panel names one. `generate-lods.mjs` filters targets
    // on any substring of their output/source/sidecar paths, so a level's own
    // output path is a filter that selects exactly that level — no second
    // selection mechanism, and no whole-model run when a single decimation is
    // what is being tuned.
    let filter = stem;
    if (body.level !== undefined && body.level !== null) {
      const state = await ladderState(stem, variantOf(url, body));
      const level = state.levels[body.level];
      if (!level?.generate || !level.model) {
        return json(res, 400, {
          problems: [`LOD ${body.level} of ${stem} is not a generated level`],
        });
      }
      filter = level.model;
    }

    if (!startRun(filter, { remesh: !!body.remesh, force: !!body.force })) {
      return json(res, 409, { problems: [`a run over ${run.model} is already going`] });
    }
    return json(res, 202, runState());
  }

  if (req.method === 'GET' && route === 'lod/generate') {
    return json(res, 200, runState());
  }

  return json(res, 404, { problems: [`no such API route: ${url.pathname}`] });
}

function runState() {
  return {
    active: run.active,
    model: run.model,
    command: run.command,
    log: run.log,
    code: run.code,
    elapsedMs: run.startedAt ? Date.now() - run.startedAt : null,
  };
}

async function serveAsset(req, res, url) {
  // Strip the /assets prefix Trunk's proxy forwards.
  const urlPath = decodeURIComponent(url.pathname).replace(/^\/assets\/?/, '');
  const filePath = path.join(ASSETS_ROOT, urlPath);

  // Refuse anything that escapes the assets root.
  if (!filePath.startsWith(ASSETS_ROOT)) {
    res.writeHead(403).end('Forbidden');
    return;
  }

  try {
    const stats = await stat(filePath);
    if (!stats.isFile()) throw new Error('not a file');
    res.writeHead(200, {
      'Content-Type': MIME[path.extname(filePath).toLowerCase()] ?? 'application/octet-stream',
      'Content-Length': stats.size,
      // Sidecars and shaders change constantly during iteration.
      'Cache-Control': 'no-cache',
    });
    createReadStream(filePath).pipe(res);
  } catch {
    // 404s are expected and meaningful: a missing rig sidecar tells Rust to
    // fall back to an identity rig rather than retry forever.
    res.writeHead(404).end('Not found');
  }
}

const assetServer = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost');
  if (url.pathname.startsWith('/api/')) {
    try {
      await handleApi(req, res, url);
    } catch (err) {
      json(res, 400, { problems: [err.message] });
    }
    return;
  }
  await serveAsset(req, res, url);
});

// Localhost only: this server writes to the working tree and runs a child
// process, so it has no business listening on a LAN interface.
assetServer.listen(ASSET_PORT, '127.0.0.1', () => {
  console.log(`[dev-viewer] assets + /api on http://127.0.0.1:${ASSET_PORT}`);
});

// A leftover viewer from a previous session is the common case here, and the
// unhandled 'error' event Node throws for it buries that in a stack trace.
assetServer.on('error', (err) => {
  if (err.code === 'EADDRINUSE') {
    console.error(
      `[dev-viewer] port ${ASSET_PORT} is already in use — another viewer is still running.\n` +
        '            Close it (or stop the stray `node scripts/dev-viewer.mjs`) and try again.',
    );
    process.exit(1);
  }
  throw err;
});

// Write the model index before Trunk starts, not only from its pre_build hook.
// Trunk canonicalises every [watch] ignore path while it is starting up, and
// assets/models/index.json is on that list *and* gitignored — so in a fresh
// checkout Trunk exits with "error taking the canonical path to the watch
// ignore path" before the hook that creates it ever runs.
await new Promise((resolve, reject) => {
  const index = spawn(process.execPath, [path.join(ROOT, 'scripts', 'generate-model-index.mjs')], {
    cwd: ROOT,
    stdio: 'inherit',
  });
  index.on('error', reject);
  index.on('close', (code) =>
    code === 0 ? resolve() : reject(new Error(`generate-model-index exited ${code}`)),
  );
});

// Both proxies (/assets/ and /api/) are declared in viewer-trunk.toml, so the
// page reaches this server through Trunk's own origin and needs no CORS.
const trunk = spawn('trunk', ['serve', '--config', 'viewer-trunk.toml'], {
  stdio: 'inherit',
  shell: process.platform === 'win32',
});

const shutdown = () => {
  trunk.kill();
  assetServer.close();
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
trunk.on('exit', (code) => {
  assetServer.close();
  process.exit(code ?? 0);
});
