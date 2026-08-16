// generate-lods.mjs — regenerate a model's LOD chain from its own sidecar
// (issue #919).
//
//   node scripts/generate-lods.mjs                 # every declared LOD output
//   node scripts/generate-lods.mjs asteroid_common_1   # just this model
//   node scripts/generate-lods.mjs --plan          # print the work, run nothing
//   node scripts/generate-lods.mjs --check         # CI: is the tree current?
//   node scripts/generate-lods.mjs --remesh        # + Blender voxel pre-pass
//   node scripts/generate-lods.mjs --adopt         # record the tree as current
//   node scripts/generate-lods.mjs --force         # accept growth the gate below refuses
//
// (`npm run lods` / `npm run lods:check` are the same two commands.)
//
// ── Where the numbers live ──────────────────────────────────────────────────
//
// Nowhere in this file. The predecessor (`generate-asteroid-lods.mjs`) carried
// a hardcoded list of four model names and a hardcoded pair of decimation
// ladders, so the script and the shipped sidecars could disagree forever and
// nothing would say so. Here the model rig sidecar is the only author: a
// `[[lod]]` level that was decimated out of another file declares how, in a
// `[lod.generate]` sub-table (src/entities/config.rs, `LodGeneration`):
//
//   [[lod]]
//   max_distance = 100.0
//   model = "assets/models/asteroid_common_1_lod1.glb"
//
//   [lod.generate]
//   source = "assets/models/asteroid_common_1.glb"
//   ratio = 0.25          # meshoptimizer target vertex ratio
//   error = 0.01          # meshoptimizer error limit
//   texture_size = 512    # max texture dimension after decimation
//
// This script reads every sidecar under assets/models, collects the levels that
// declare one of those blocks, and runs `simplify` then `resize` through the
// PINNED @gltf-transform/cli devDependency. A level with no `[lod.generate]` is
// hand-authored and is never touched.
//
// Three sidecars (small / large / cosmetic) share one generated `.glb` per
// asteroid, so the collector de-duplicates by output path and treats two
// sidecars claiming the same output with different parameters as an error
// rather than a race between whichever ran last.
//
// ── Drift, and why it is a manifest rather than a rebuild ────────────────────
//
// `--check` is what CI runs, and it never invokes gltf-transform. Byte-for-byte
// reproducibility across machines is not something meshoptimizer promises — a
// different CLI build can produce a valid, differently-packed GLB from the same
// input — so "regenerate and diff" would go red for reasons that are not drift.
//
// Instead every generated file is recorded in scripts/lod-manifest.toml with
// the hash of its source, the parameters it was made with, and the hash and
// byte size of the output. Re-hashing three files per output catches all three
// ways the tree can silently rot:
//
//   - the source .glb was replaced and nobody regenerated  → source_sha256
//   - the ratio/error/size was retuned and nobody regenerated → params
//   - a generated .glb was hand-edited or truncated        → output_sha256
//
// The parameters are stored as a readable inline table rather than a hash of
// one: it is four small numbers, and a reviewer seeing `ratio 0.25 → 0.05` in
// the diff learns more than they would from a changed hex string.
//
// The manifest lives under scripts/ and not beside the models because every
// `.toml` in assets/models is a rig sidecar, asserted as such over the whole
// directory by `every_shipped_sidecar_parses_strictly`. It is build metadata,
// so it does not ship to the browser either.
//
// File SIZE is not judged here. `src/perf/assets.rs` (issue #868) already owns
// the byte measurement, and a test there asserts the `output_bytes` recorded
// below agree with the inventory it takes — one measurement, two readers.
// Triangle and texture-count budgets belong to issue #905, which measures them
// through Bevy's own loader; deliberately not duplicated here.
//
// ── Prerequisites ───────────────────────────────────────────────────────────
//
//   npm install     # brings the pinned @gltf-transform/cli + smol-toml
//
// Regeneration is a local command, on purpose: it rewrites binaries under
// assets/ and needs the CLI. CI only re-hashes. (`overrides.sharp` in
// package.json is load-bearing for the resize step — see the note there.)
//
// ── Known: asteroid_common_4 is a stubborn mesh ──────────────────────────────
//
// Regenerating its ladder today produces levels several times LARGER than the
// ones in the tree: 169k vertices over 125k triangles is a mesh that is mostly
// split vertices, and meshoptimizer stalls around 67% no matter how loose the
// error bound goes. The shipped `_lod1`/`_lod2` for it predate this script and
// remain the better files, so they are recorded as-is (`--adopt`) rather than
// replaced. This is exactly the case the Blender voxel pre-pass below exists
// for; a level with no `remesh_voxel_size` that regenerates larger than its
// adopted baseline is refused outright (see the growth gate below) rather
// than merely reported, so it cannot land quietly. `--force` overrides it for
// the rare case where a bigger file really is the right call.

import { readdir, readFile, writeFile, mkdir, rm } from 'node:fs/promises';
import { existsSync, statSync, readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import path from 'node:path';
import os from 'node:os';
import { pathToFileURL } from 'node:url';
import { parse as parseToml } from 'smol-toml';

const execFileAsync = promisify(execFile);

export const MODELS_DIR = 'assets/models';
export const MANIFEST_PATH = 'scripts/lod-manifest.toml';
export const MANIFEST_VERSION = 1;
/// The intermediate a Blender voxel pre-pass writes next to its source. Checked
/// in, because the pre-pass needs Blender and the decimation must not.
export const REMESH_SUFFIX = '.remesh.glb';
export const BLENDER_SCRIPT = 'scripts/blender-voxel-remesh.py';

// ── Pure: sidecar declarations → a work plan ────────────────────────────────

/** Where a voxel-remeshed intermediate for `source` is written. */
export function remeshPath(source) {
  return source.replace(/\.glb$/i, REMESH_SUFFIX);
}

/**
 * The largest `texture_size` any level cut from `remeshFile` asks for — the
 * resolution above which that intermediate's textures buy nothing.
 *
 * DERIVED from the sidecars rather than fixed at a number, because the shipped
 * ladders disagree: most models cut 256 and 128 from their remesh, but
 * `dynasty_battleship_lod1` authors `texture_size = 512`. A hardcoded 256 would
 * have quietly halved the one level in the tree that asks for more.
 *
 * The MAXIMUM over every consumer is what makes a single cap safe to apply to a
 * file two levels share: capping to either level's own size would starve the
 * other. `null` means leave the file alone — either nothing is cut from it, or
 * some consumer declares no `texture_size` at all and therefore ships the
 * intermediate's textures exactly as they are.
 */
export function remeshTextureCap(targets, remeshFile) {
  const consumers = targets.filter((t) => t.effectiveSource === remeshFile);
  if (!consumers.length) return null;
  if (consumers.some((t) => t.params.textureSize === null)) return null;
  return Math.max(...consumers.map((t) => t.params.textureSize));
}

/**
 * One line describing a target's parameters, used to compare a manifest entry
 * against what the sidecars now say. Every key is present even when unset, so
 * *removing* a parameter reads as a change rather than as a match.
 */
export function canonicalParams(params) {
  const show = (v) => (v === null || v === undefined ? 'none' : String(v));
  return [
    `ratio=${show(params.ratio)}`,
    `error=${show(params.error)}`,
    `texture_size=${show(params.textureSize)}`,
    `remesh_voxel_size=${show(params.remeshVoxelSize)}`,
  ].join(' ');
}

/**
 * Collect every generated output declared by a set of parsed sidecars.
 *
 * `sidecars` is `[{ path, doc }]` — the parse happens at the edge so this stays
 * a pure function over data. Returns targets sorted by output path (so a run,
 * a plan and a manifest always list the same work in the same order) plus the
 * authoring errors found along the way; a caller with errors should refuse to
 * run rather than generate the subset that happened to be valid.
 */
export function collectTargets(sidecars) {
  const byOutput = new Map();
  const errors = [];
  const ordered = [...sidecars].sort((a, b) => a.path.localeCompare(b.path));

  for (const { path: sidecarPath, doc } of ordered) {
    const levels = Array.isArray(doc?.lod) ? doc.lod : [];
    // A level that omits `generate.source` decimates the ladder's own near
    // level — the full-detail model the whole chain is derived from.
    const firstGlb = levels.find((l) => typeof l?.model === 'string')?.model ?? null;

    levels.forEach((level, index) => {
      const gen = level?.generate;
      if (!gen) return;
      const where = `${sidecarPath} [[lod]] #${index}`;

      const output = level.model;
      if (typeof output !== 'string') {
        errors.push(`${where}: declares [lod.generate] but names no model to generate`);
        return;
      }
      const source = typeof gen.source === 'string' ? gen.source : firstGlb;
      if (!source) {
        errors.push(`${where}: no generate.source, and the ladder has no GLB level to derive one from`);
        return;
      }
      if (source === output) {
        errors.push(`${where}: source and output are both ${output}`);
        return;
      }

      const params = {
        ratio: gen.ratio ?? null,
        error: gen.error ?? null,
        textureSize: gen.texture_size ?? null,
        remeshVoxelSize: gen.remesh_voxel_size ?? null,
      };
      if (params.ratio === null && params.textureSize === null) {
        errors.push(`${where}: neither ratio nor texture_size — the step would copy its source`);
      }
      if (params.ratio !== null && !(params.ratio > 0 && params.ratio < 1)) {
        errors.push(`${where}: ratio ${params.ratio} must be between 0 and 1 (exclusive)`);
      }
      if (params.error !== null && !(params.error > 0)) {
        errors.push(`${where}: error ${params.error} must be greater than 0`);
      }
      if (
        params.textureSize !== null &&
        !(Number.isInteger(params.textureSize) && params.textureSize > 0)
      ) {
        errors.push(`${where}: texture_size ${params.textureSize} must be a positive whole number of pixels`);
      }
      if (params.remeshVoxelSize !== null && !(params.remeshVoxelSize > 0)) {
        errors.push(`${where}: remesh_voxel_size ${params.remeshVoxelSize} must be greater than 0`);
      }

      const existing = byOutput.get(output);
      if (existing) {
        // The three rig variants of one asteroid share one generated file. They
        // may (and do) declare it repeatedly; they may not disagree about it.
        if (
          existing.source !== source ||
          canonicalParams(existing.params) !== canonicalParams(params)
        ) {
          errors.push(
            `${output}: declared differently by ${existing.declaredBy[0]} and ${sidecarPath} — ` +
              `variants of one model share one generated file and must agree on how it is made`,
          );
        } else if (!existing.declaredBy.includes(sidecarPath)) {
          existing.declaredBy.push(sidecarPath);
        }
        return;
      }
      byOutput.set(output, { output, source, params, declaredBy: [sidecarPath] });
    });
  }

  const targets = [...byOutput.values()].sort((a, b) => a.output.localeCompare(b.output));
  for (const target of targets) {
    target.declaredBy.sort();
    // What the decimation actually reads: the checked-in voxel-remeshed
    // intermediate when one is declared, otherwise the source itself.
    target.effectiveSource =
      target.params.remeshVoxelSize === null ? target.source : remeshPath(target.source);
  }
  return { targets, errors };
}

/** Positional filters match a model name, an output path or a sidecar path. */
export function matchesFilter(target, filters) {
  if (!filters.length) return true;
  const haystack = [target.output, target.source, ...target.declaredBy].join(' ');
  return filters.some((f) => haystack.includes(f));
}

/**
 * The gltf-transform invocations for one target, in order.
 *
 * `cli` is the argv prefix that runs the pinned CLI (an array, so tests can
 * pass a readable placeholder). `tmpDir` holds the intermediate between
 * simplify and resize; it is a parameter rather than an `mkdtemp` call so the
 * plan is a deterministic value that can be asserted without running anything.
 */
export function planSteps(target, { cli, tmpDir }) {
  const { ratio, error, textureSize } = target.params;
  const steps = [];
  const resizing = textureSize !== null;
  const simplified = resizing ? path.posix.join(tmpDir, 'simplified.glb') : target.output;

  if (ratio !== null) {
    const argv = [...cli, 'simplify', target.effectiveSource, simplified, '--ratio', String(ratio)];
    if (error !== null) argv.push('--error', String(error));
    steps.push({ label: `simplify ratio=${ratio} error=${error ?? 'default'}`, argv });
  }
  if (resizing) {
    const input = ratio !== null ? simplified : target.effectiveSource;
    steps.push({
      label: `resize ${textureSize}px`,
      argv: [
        ...cli,
        'resize',
        input,
        target.output,
        '--width',
        String(textureSize),
        '--height',
        String(textureSize),
      ],
    });
  }
  return steps;
}

/**
 * The Blender pre-pass for a target that declares one, or `null`.
 *
 * Optional throughout: no shipped ladder needs it, and the main command must
 * work on a machine with no Blender at all. It exists for meshes that decimate
 * badly — non-manifold geometry, overlapping shells, holes — where a voxel
 * remesh first rebuilds a watertight surface that meshoptimizer can reduce
 * predictably. The intermediate it writes is checked in, so only the person
 * who re-runs the pre-pass needs Blender.
 */
export function remeshStep(target, { blender, script = BLENDER_SCRIPT }) {
  const voxel = target.params.remeshVoxelSize;
  if (voxel === null) return null;
  return {
    label: `voxel remesh ${voxel} → ${remeshPath(target.source)}`,
    argv: [
      blender,
      '--background',
      '--factory-startup',
      '--python',
      script,
      '--',
      target.source,
      remeshPath(target.source),
      String(voxel),
    ],
  };
}

/**
 * Where to look for Blender, in order. Pure so the lookup order is testable
 * without a Blender install.
 *
 * `PHOENIX_BLENDER` wins, then whatever `blender` resolves to on PATH, then —
 * on Windows, where the installer does not touch PATH — the versioned install
 * directories `C:\Program Files\Blender Foundation\Blender <version>\blender.exe`,
 * newest first. `installedDirs` is the (already-listed) contents of the
 * Blender Foundation directory; the caller supplies it, or an empty list.
 */
export function blenderCandidates({ env = {}, platform = process.platform, installedDirs = [] } = {}) {
  const candidates = [];
  if (env.PHOENIX_BLENDER) candidates.push(env.PHOENIX_BLENDER);
  candidates.push('blender');
  if (platform === 'win32') {
    const root = path.win32.join(
      env.ProgramFiles || 'C:\\Program Files',
      'Blender Foundation',
    );
    const versioned = [...installedDirs]
      .filter((d) => d.startsWith('Blender'))
      .sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
    for (const dir of versioned) {
      candidates.push(path.win32.join(root, dir, 'blender.exe'));
    }
  }
  return candidates;
}

// ── Pure: the manifest ──────────────────────────────────────────────────────

export function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

/**
 * A TOML float literal: whole numbers still need a decimal point.
 *
 * Exported because the viewer's LOD panel writes the same `[lod.generate]`
 * numbers back into the same sidecars (scripts/viewer-lods.mjs). Two spellings
 * of `50` would make every hand-edited ladder churn the moment the panel
 * touched it.
 */
export function tomlFloat(value) {
  return Number.isInteger(value) ? `${value}.0` : String(value);
}

function paramsTable(params) {
  const fields = [];
  if (params.ratio !== null) fields.push(`ratio = ${tomlFloat(params.ratio)}`);
  if (params.error !== null) fields.push(`error = ${tomlFloat(params.error)}`);
  if (params.textureSize !== null) fields.push(`texture_size = ${params.textureSize}`);
  if (params.remeshVoxelSize !== null) {
    fields.push(`remesh_voxel_size = ${tomlFloat(params.remeshVoxelSize)}`);
  }
  return `{ ${fields.join(', ')} }`;
}

/**
 * One manifest record. `observed` carries the hashes taken from disk:
 * `{ sourceSha256, outputSha256, outputBytes, originSha256 }` — `origin*` only
 * when a remesh pre-pass sits between the source and the decimation.
 */
export function manifestEntry(target, observed) {
  const entry = {
    path: target.output,
    source: target.effectiveSource,
    source_sha256: observed.sourceSha256,
    params: canonicalParams(target.params),
    paramsValues: target.params,
    output_sha256: observed.outputSha256,
    output_bytes: observed.outputBytes,
    declared_by: [...target.declaredBy],
  };
  if (target.effectiveSource !== target.source) {
    entry.origin = target.source;
    entry.origin_sha256 = observed.originSha256 ?? null;
  }
  return entry;
}

/** Render the manifest. Sorted by output path; stable across machines. */
export function formatManifest(entries) {
  const lines = [
    '# Generated by scripts/generate-lods.mjs (issue #919) — do not edit by hand.',
    '#',
    '# One record per generated LOD .glb: what it was made from, what it was made',
    '# with, and what came out. `node scripts/generate-lods.mjs --check` re-hashes',
    '# all three and fails if the tree has drifted from what the sidecars declare.',
    '# A record adopted from files already on disk (`--adopt`) is a baseline like',
    '# any other: the growth gate measures the next regeneration against it exactly',
    '# as it would a record this script produced itself.',
    '',
    `version = ${MANIFEST_VERSION}`,
  ];
  for (const entry of [...entries].sort((a, b) => a.path.localeCompare(b.path))) {
    lines.push('');
    lines.push('[[output]]');
    lines.push(`path = ${JSON.stringify(entry.path)}`);
    lines.push(`source = ${JSON.stringify(entry.source)}`);
    lines.push(`source_sha256 = ${JSON.stringify(entry.source_sha256 ?? '')}`);
    if (entry.origin) {
      lines.push(`origin = ${JSON.stringify(entry.origin)}`);
      lines.push(`origin_sha256 = ${JSON.stringify(entry.origin_sha256 ?? '')}`);
    }
    lines.push(`params = ${paramsTable(entry.paramsValues)}`);
    lines.push(`output_sha256 = ${JSON.stringify(entry.output_sha256 ?? '')}`);
    lines.push(`output_bytes = ${entry.output_bytes ?? 0}`);
    lines.push(`declared_by = [${entry.declared_by.map((d) => JSON.stringify(d)).join(', ')}]`);
  }
  return `${lines.join('\n')}\n`;
}

/** Read a manifest back into `manifestEntry`-shaped records. */
export function parseManifest(text) {
  const doc = parseToml(text);
  const outputs = Array.isArray(doc.output) ? doc.output : [];
  return outputs.map((o) => ({
    path: o.path,
    source: o.source,
    source_sha256: o.source_sha256,
    origin: o.origin,
    origin_sha256: o.origin_sha256,
    params: canonicalParams({
      ratio: o.params?.ratio ?? null,
      error: o.params?.error ?? null,
      textureSize: o.params?.texture_size ?? null,
      remeshVoxelSize: o.params?.remesh_voxel_size ?? null,
    }),
    paramsValues: {
      ratio: o.params?.ratio ?? null,
      error: o.params?.error ?? null,
      textureSize: o.params?.texture_size ?? null,
      remeshVoxelSize: o.params?.remesh_voxel_size ?? null,
    },
    output_sha256: o.output_sha256,
    output_bytes: Number(o.output_bytes ?? 0),
    declared_by: Array.isArray(o.declared_by) ? o.declared_by : [],
  }));
}

/**
 * Compare recorded state against what the sidecars declare and what is on disk.
 *
 * `observed` is `[{ target, sourceSha256, outputSha256, outputBytes }]`. Every
 * finding names the file, what changed, and (in the caller's summary) the one
 * command that fixes it. Findings are sorted for a stable CI log.
 */
export function compareManifest(entries, observed) {
  const findings = [];
  const recorded = new Map(entries.map((e) => [e.path, e]));

  for (const item of observed) {
    const { target } = item;
    const entry = recorded.get(target.output);
    recorded.delete(target.output);
    if (!entry) {
      findings.push({
        output: target.output,
        kind: 'unrecorded',
        detail: `declared by ${target.declaredBy[0]} but absent from the manifest`,
      });
      continue;
    }
    if (item.outputSha256 === null) {
      findings.push({ output: target.output, kind: 'missing-output', detail: 'the generated file is not on disk' });
      continue;
    }
    if (item.sourceSha256 === null) {
      findings.push({
        output: target.output,
        kind: 'missing-source',
        detail: `${target.effectiveSource} is not on disk`,
      });
      continue;
    }
    if (entry.source !== target.effectiveSource) {
      findings.push({
        output: target.output,
        kind: 'source-repointed',
        detail: `made from ${entry.source}, sidecar now says ${target.effectiveSource}`,
      });
    } else if (entry.source_sha256 !== item.sourceSha256) {
      findings.push({
        output: target.output,
        kind: 'source-changed',
        detail: `${target.effectiveSource} changed since this level was generated`,
      });
    }
    const params = canonicalParams(target.params);
    if (entry.params !== params) {
      findings.push({
        output: target.output,
        kind: 'params-changed',
        detail: `made with [${entry.params}], sidecar now says [${params}]`,
      });
    }
    if (entry.output_sha256 !== item.outputSha256) {
      findings.push({
        output: target.output,
        kind: 'output-changed',
        detail: 'the generated file on disk is not the one that was recorded',
      });
    }
  }

  for (const orphan of recorded.values()) {
    findings.push({
      output: orphan.path,
      kind: 'orphaned',
      detail: 'recorded in the manifest but no sidecar declares it any more',
    });
  }

  return findings.sort((a, b) => a.output.localeCompare(b.output) || a.kind.localeCompare(b.kind));
}

export function describeFindings(findings) {
  return findings.map((f) => `  ${f.output}: ${f.kind} — ${f.detail}`).join('\n');
}

function formatBytes(bytes) {
  if (bytes === null || bytes === undefined) return '—';
  return bytes >= 1024 * 1024
    ? `${(bytes / 1024 / 1024).toFixed(2)} MB`
    : `${(bytes / 1024).toFixed(1)} KB`;
}

/**
 * How each regenerated output's size moved against the record it replaced.
 *
 * A warning, never a failure: retuning a level upward is a legitimate choice.
 * But a decimated level that comes out *bigger* than the one it replaces is
 * almost always a mesh meshoptimizer could not reduce — split vertices,
 * non-manifold shells — and that is worth saying out loud at the moment it
 * happens rather than discovering it as a download regression. `grew` is the
 * flag; the byte figures themselves belong to `src/perf/assets.rs` (#868),
 * which measures the whole tree, and triangle counts to issue #905.
 */
export function sizeReport(previous, observed) {
  const before = new Map(previous.map((e) => [e.path, e.output_bytes]));
  return observed.map((item) => {
    const bytes = item.outputBytes;
    const previousBytes = before.has(item.target.output) ? before.get(item.target.output) : null;
    const grew = previousBytes !== null && bytes !== null && bytes > previousBytes;
    return {
      output: item.target.output,
      target: item.target,
      bytes,
      previousBytes,
      grew,
      // Whether this level already names its own remedy for a mesh that
      // regenerates larger — see `blockedGrowth` below.
      remeshDeclared: item.target.params ? item.target.params.remeshVoxelSize !== null : false,
      line:
        `  ${item.target.output}: ${formatBytes(previousBytes)} → ${formatBytes(bytes)}` +
        (grew ? '  ← LARGER than the file it replaced' : ''),
    };
  });
}

/**
 * Which of `sizeReport`'s entries the growth gate refuses to record, absent
 * `{ force: true }`.
 *
 * A level that declares `remesh_voxel_size` is exempt even when it grows: the
 * voxel pre-pass IS the assigned remedy for a mesh meshoptimizer cannot
 * shrink, so a level that already asks for it has already taken the
 * intended path — gating it a second time here would leave the person
 * looking at the failure with no further step to take. A level with no such
 * declaration has no assigned remedy yet, so growth there is refused instead
 * of merely logged (`sizeReport`'s `line` already carries the warning either
 * way, gated or not).
 */
export function blockedGrowth(sizes, { force = false } = {}) {
  if (force) return [];
  return sizes.filter((s) => s.grew && !s.remeshDeclared);
}

/** One line per blocked output, naming the model and the sizes involved. */
export function describeBlockedGrowth(blocked) {
  return blocked
    .map(
      (b) =>
        `  ${b.output}: ${formatBytes(b.previousBytes)} → ${formatBytes(b.bytes)} — grew past its recorded baseline`,
    )
    .join('\n');
}

// ── Impure: disk, processes, CLI ────────────────────────────────────────────

const CLI_ENTRY = 'node_modules/@gltf-transform/cli/bin/cli.js';

/**
 * The argv prefix that runs the PINNED CLI. Resolved through node against the
 * installed package rather than `npx @gltf-transform/cli`, which would fetch
 * whatever version is current the day it runs — the exact way the previous
 * script could stop reproducing its own outputs.
 */
function gltfTransformCli(root) {
  const entry = path.join(root, CLI_ENTRY);
  if (!existsSync(entry)) {
    const pinned = JSON.parse(readFileSync(path.join(root, 'package.json'), 'utf8'))
      .devDependencies['@gltf-transform/cli'];
    throw new Error(
      `@gltf-transform/cli@${pinned} is not installed — run \`npm install\` before generating LODs`,
    );
  }
  return [process.execPath, entry];
}

async function readSidecars(root) {
  const dir = path.join(root, MODELS_DIR);
  const files = (await readdir(dir)).filter((f) => f.endsWith('.toml')).sort();
  const sidecars = [];
  for (const file of files) {
    const rel = `${MODELS_DIR}/${file}`;
    const text = await readFile(path.join(dir, file), 'utf8');
    try {
      sidecars.push({ path: rel, doc: parseToml(text) });
    } catch (err) {
      throw new Error(`${rel}: ${err.message}`);
    }
  }
  return sidecars;
}

function hashFileSync(file) {
  if (!existsSync(file)) return null;
  return sha256(readFileSync(file));
}

function observe(root, targets) {
  return targets.map((target) => {
    const output = path.join(root, target.output);
    return {
      target,
      sourceSha256: hashFileSync(path.join(root, target.effectiveSource)),
      originSha256:
        target.effectiveSource === target.source
          ? null
          : hashFileSync(path.join(root, target.source)),
      outputSha256: hashFileSync(output),
      outputBytes: existsSync(output) ? statSync(output).size : null,
    };
  });
}

/**
 * Shrink a remesh intermediate's textures to `cap` pixels, in place.
 *
 * A voxel remesh is a GEOMETRY pass: Blender rebuilds the surface and carries
 * the source's materials across untouched, so the intermediate inherits the
 * full-size base-colour and normal maps of the hero model — 2048px on both
 * couriers and on the starbase. Nothing ever sees a pixel of that. Every level
 * cut from a remesh runs `resize` down to its own `texture_size` first, so
 * everything above the largest of those is decoded, re-encoded and committed
 * purely to be thrown away by the next step: `alliance_starbase.remesh.glb`
 * carried 6.2 MB of texture to feed a 256px cut and a 128px one.
 *
 * Images already within the cap are left BYTE-FOR-BYTE alone rather than
 * re-encoded, so a model whose textures were always small keeps its
 * intermediate's hash and nothing downstream of it churns.
 *
 * PNG throughout, never WebP/KTX2 — Bevy's glTF loader rejects
 * EXT_texture_webp, the same constraint scripts/optimise-base.mjs works under.
 *
 * The two imports are dynamic on purpose: `--check` is what CI runs, it never
 * touches a texture, and it should not pay to load sharp and the glTF core to
 * re-hash nine files.
 *
 * Returns `{ resized, before, after }` — the count and the texture bytes either
 * side, for the caller's log.
 */
export async function capRemeshTextures(file, cap) {
  const { NodeIO } = await import('@gltf-transform/core');
  const { default: sharp } = await import('sharp');
  const io = new NodeIO();
  const document = await io.read(file);
  let resized = 0;
  let before = 0;
  let after = 0;
  for (const texture of document.getRoot().listTextures()) {
    const image = texture.getImage();
    if (!image) continue;
    before += image.length;
    const meta = await sharp(Buffer.from(image)).metadata();
    if (meta.width <= cap && meta.height <= cap) {
      after += image.length;
      continue;
    }
    const shrunk = await sharp(Buffer.from(image))
      .resize(cap, cap, { fit: 'inside', withoutEnlargement: true })
      .png()
      .toBuffer();
    texture.setImage(new Uint8Array(shrunk));
    texture.setMimeType('image/png');
    after += shrunk.length;
    resized += 1;
  }
  if (resized) await io.write(file, document);
  return { resized, before, after };
}

async function resolveBlender(root) {
  let installedDirs = [];
  const foundation = path.win32.join(
    process.env.ProgramFiles || 'C:\\Program Files',
    'Blender Foundation',
  );
  if (process.platform === 'win32' && existsSync(foundation)) {
    installedDirs = await readdir(foundation);
  }
  for (const candidate of blenderCandidates({ env: process.env, installedDirs })) {
    try {
      await execFileAsync(candidate, ['--version']);
      return candidate;
    } catch {
      /* try the next one */
    }
  }
  throw new Error(
    'no Blender found — set PHOENIX_BLENDER, put `blender` on PATH, or install it under ' +
      '"C:\\Program Files\\Blender Foundation\\Blender <version>". The voxel pre-pass is ' +
      'optional; drop --remesh to generate from the checked-in sources instead.',
  );
}

function parseArgs(argv) {
  const flags = new Set(argv.filter((a) => a.startsWith('--')));
  const unknown = [...flags].filter(
    (f) => !['--check', '--plan', '--adopt', '--remesh', '--force'].includes(f),
  );
  return {
    check: flags.has('--check'),
    plan: flags.has('--plan'),
    adopt: flags.has('--adopt'),
    remesh: flags.has('--remesh'),
    force: flags.has('--force'),
    filters: argv.filter((a) => !a.startsWith('--')),
    unknown,
  };
}

async function main() {
  const root = process.cwd();
  const cli = parseArgs(process.argv.slice(2));
  if (cli.unknown.length) {
    console.error(`[generate-lods] unknown flag(s): ${cli.unknown.join(', ')}`);
    process.exit(2);
  }

  const { targets: allTargets, errors } = collectTargets(await readSidecars(root));
  if (errors.length) {
    console.error('[generate-lods] the sidecars do not agree on what to generate:');
    for (const e of errors) console.error(`  ${e}`);
    process.exit(1);
  }
  if (!allTargets.length) {
    console.error('[generate-lods] no sidecar declares a [lod.generate] level — nothing to do');
    return;
  }

  const selected = allTargets.filter((t) => matchesFilter(t, cli.filters));
  if (!selected.length) {
    console.error(`[generate-lods] no declared LOD output matches ${cli.filters.join(' ')}`);
    process.exit(1);
  }

  const manifestFile = path.join(root, MANIFEST_PATH);
  const recorded = existsSync(manifestFile)
    ? parseManifest(await readFile(manifestFile, 'utf8'))
    : [];

  // ── --check: re-hash, compare, gate. No CLI, no writes. ──────────────────
  if (cli.check) {
    const findings = compareManifest(recorded, observe(root, selected));
    if (findings.length) {
      console.error(
        `[generate-lods] ${findings.length} LOD output(s) have drifted from their sidecars:`,
      );
      console.error(describeFindings(findings));
      console.error(
        '\n  Regenerate locally with `npm run lods` (or `node scripts/generate-lods.mjs <model>`)\n' +
          `  and commit the rebuilt .glb files together with ${MANIFEST_PATH}.`,
      );
      process.exit(1);
    }
    console.error(`[generate-lods] ${selected.length} LOD output(s) up to date with their sidecars`);
    return;
  }

  // ── --plan: print the work, run nothing. ─────────────────────────────────
  if (cli.plan) {
    for (const target of selected) {
      console.log(`${target.output}  ← ${target.effectiveSource}  [${canonicalParams(target.params)}]`);
      console.log(`    declared by ${target.declaredBy.join(', ')}`);
      const remesh = remeshStep(target, { blender: 'blender' });
      const steps = remesh
        ? [remesh, ...planSteps(target, { cli: ['gltf-transform'], tmpDir: '<tmp>' })]
        : planSteps(target, { cli: ['gltf-transform'], tmpDir: '<tmp>' });
      for (const step of steps) console.log(`    ${step.label}`);
    }
    return;
  }

  // ── --adopt: record what is already on disk as current. ──────────────────
  //
  // The one-time baseline for a tree whose LODs predate this script (and the
  // escape hatch after a deliberate hand-fix). It blesses whatever is there —
  // so it is never what CI runs, and never the answer to a red --check unless
  // you have just looked at the files.
  if (cli.adopt) {
    const observed = observe(root, selected);
    const missing = observed.filter((o) => o.outputSha256 === null);
    if (missing.length) {
      console.error('[generate-lods] cannot adopt outputs that are not on disk:');
      for (const m of missing) console.error(`  ${m.target.output}`);
      process.exit(1);
    }
    await writeManifest(root, allTargets, recorded, observed);
    console.error(`[generate-lods] recorded ${observed.length} existing output(s) in ${MANIFEST_PATH}`);
    return;
  }

  // ── Default: generate. ───────────────────────────────────────────────────
  const gltf = gltfTransformCli(root);
  const blender = cli.remesh && selected.some((t) => t.params.remeshVoxelSize !== null)
    ? await resolveBlender(root)
    : null;

  for (const target of selected) {
    console.error(`\n[generate-lods] ${target.output}`);
    console.error(`  from ${target.effectiveSource}  [${canonicalParams(target.params)}]`);

    const remesh = remeshStep(target, { blender });
    if (remesh) {
      if (cli.remesh) {
        console.error(`  ${remesh.label}`);
        await execFileAsync(remesh.argv[0], remesh.argv.slice(1));
        // Blender hands back the source's full-size materials on a geometry
        // pass; cap them at what this intermediate's own consumers actually
        // cut. `allTargets`, not `selected` — the cap has to account for every
        // level that shares the file, including ones this run filtered out.
        const cap = remeshTextureCap(allTargets, target.effectiveSource);
        if (cap !== null) {
          const capped = await capRemeshTextures(
            path.join(root, target.effectiveSource),
            cap,
          );
          console.error(
            `  cap textures at ${cap}px — ${capped.resized} resized, ` +
              `${formatBytes(capped.before)} → ${formatBytes(capped.after)} of texture`,
          );
        }
      } else if (!existsSync(path.join(root, target.effectiveSource))) {
        console.error(
          `  ${target.effectiveSource} is missing — this level declares a voxel pre-pass; ` +
            're-run with --remesh (needs Blender) to rebuild it',
        );
        process.exit(1);
      }
    }

    const tmpDir = path.join(os.tmpdir(), `phoenix-lod-${path.basename(target.output, '.glb')}`);
    await mkdir(tmpDir, { recursive: true });
    try {
      for (const step of planSteps(target, { cli: gltf, tmpDir: tmpDir.split(path.sep).join('/') })) {
        console.error(`  ${step.label}`);
        await execFileAsync(step.argv[0], step.argv.slice(1));
      }
    } finally {
      await rm(tmpDir, { recursive: true, force: true });
    }
  }

  const observed = observe(root, selected);
  const sizes = sizeReport(recorded, observed);
  console.error(`\n[generate-lods] regenerated ${observed.length} output(s):`);
  for (const size of sizes) console.error(size.line);
  if (sizes.some((s) => s.grew)) {
    console.error(
      '\n  One or more levels grew. A step that cannot be decimated further is the\n' +
        '  stubborn-mesh case: consider the Blender voxel pre-pass (remesh_voxel_size\n' +
        `  in [lod.generate], then --remesh) — see ${BLENDER_SCRIPT}.`,
    );
  }

  // ── Growth gate: a level with no assigned remedy that regenerates larger
  // than its recorded baseline is a hard failure, not a warning — unless
  // --force says the bigger file is the right call. The blocked output(s)
  // keep their previous manifest record so a batch run over many models
  // still lands the fixes it found for everything else.
  const wouldBlock = blockedGrowth(sizes);
  const blocked = cli.force ? [] : wouldBlock;
  if (blocked.length) {
    console.error(
      `\n[generate-lods] refusing to record ${blocked.length} output(s) that grew past their\n` +
        '  recorded baseline with no remesh_voxel_size declared:',
    );
    console.error(describeBlockedGrowth(blocked));
    console.error(
      '\n  This is the stubborn-mesh case the Blender voxel pre-pass exists for: declare\n' +
        '  remesh_voxel_size in [lod.generate] for the model(s) above and re-run with\n' +
        `  --remesh (see ${BLENDER_SCRIPT}). Pass --force to record the larger output(s) anyway.`,
    );
    const blockedOutputs = new Set(blocked.map((b) => b.output));
    await writeManifest(
      root,
      allTargets,
      recorded,
      observed.filter((o) => !blockedOutputs.has(o.target.output)),
    );
    console.error(
      `[generate-lods] updated ${MANIFEST_PATH} — kept the previous baseline for ${blocked.length} blocked output(s)`,
    );
    process.exit(1);
  }
  if (cli.force && wouldBlock.length) {
    console.error(
      `\n[generate-lods] --force: recording ${wouldBlock.length} output(s) that grew past their recorded baseline anyway.`,
    );
  }

  await writeManifest(root, allTargets, recorded, observed);
  console.error(`[generate-lods] updated ${MANIFEST_PATH}`);
}

/**
 * Write the manifest, keeping records for outputs this run did not touch and
 * dropping records for outputs no sidecar declares any more. `allTargets` is
 * the unfiltered list, so a single-model run never silently prunes the rest.
 */
async function writeManifest(root, allTargets, recorded, observed) {
  const declared = new Set(allTargets.map((t) => t.output));
  const entries = new Map(
    recorded.filter((e) => declared.has(e.path)).map((e) => [e.path, e]),
  );
  for (const item of observed) {
    entries.set(item.target.output, manifestEntry(item.target, item));
  }
  await writeFile(path.join(root, MANIFEST_PATH), formatManifest([...entries.values()]));
}

// Guard the CLI entry so importing this module (from tests, or `node -e`) never
// touches the filesystem — same contract as scripts/balance-runs.mjs.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    console.error(`[generate-lods] ${err.message}`);
    process.exit(1);
  });
}
