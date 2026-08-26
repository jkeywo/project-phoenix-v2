// capture-billboards.mjs - capture and verify sidecar-owned billboard atlases.
//
//   node scripts/capture-billboards.mjs                 # recapture every atlas
//   node scripts/capture-billboards.mjs alliance_cruiser # one model/output
//   node scripts/capture-billboards.mjs --check         # CI: hash-only currency gate
//   node scripts/capture-billboards.mjs --adopt         # record committed PNGs
//
// A `[lod.capture]` block is the complete recipe. The native renderer is only
// needed for a recapture; `--check` and `--adopt` inspect committed files and
// therefore work on CI and on machines without a GPU or capture binary.

import { readdir, readFile, writeFile } from 'node:fs/promises';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { parse as parseToml } from 'smol-toml';
import { ladderFromDoc, replaceLadder } from './viewer-lods.mjs';

export const MODELS_DIR = 'assets/models';
export const CAPTURE_MANIFEST_PATH = 'scripts/lod-capture-manifest.toml';
export const CAPTURE_MANIFEST_VERSION = 1;
// Bump whenever framing, lighting, packing, or another renderer-side semantic
// changes in a way that requires every atlas to be recaptured.
export const CAPTURE_RECIPE_VERSION = 1;
export const IDENTITY_BASE = Object.freeze({
  offset: Object.freeze([0, 0, 0]),
  rotation: Object.freeze([0, 0, 0]),
  scale: Object.freeze([1, 1, 1]),
});

const BIN = process.env.PHOENIX_CAPTURE_BIN ?? (process.platform === 'win32'
  ? 'target/debug/capture-billboard.exe'
  : 'target/debug/capture-billboard');

const round = (x) => Math.round(x * 1e4) / 1e4;
const show = (value) => (value === null || value === undefined ? 'none' : String(value));

export function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

/** Repo-relative paths are stored with `/` on every host. */
export function canonicalRepoPath(value, where, errors = []) {
  if (typeof value !== 'string' || !value.length) {
    errors.push(`${where} must be a non-empty repo-relative path`);
    return null;
  }
  const slashed = value.replaceAll('\\', '/');
  if (
    path.posix.isAbsolute(slashed) ||
    path.win32.isAbsolute(value) ||
    slashed.split('/').includes('..')
  ) {
    errors.push(`${where} must stay inside the repository (absolute paths and .. are forbidden)`);
    return null;
  }
  const canonical = path.posix.normalize(slashed).replace(/^\.\//, '');
  if (!canonical || canonical === '.') {
    errors.push(`${where} must name a file`);
    return null;
  }
  return canonical;
}

function normaliseVector(value, fallback, where, errors) {
  if (value === undefined || value === null) return [...fallback];
  if (!Array.isArray(value) || value.length !== 3 || value.some((v) => !Number.isFinite(v))) {
    errors.push(`${where} must be an array of three finite numbers`);
    return [...fallback];
  }
  return value.map(Number);
}

/** Fill the runtime's sparse `[base]` defaults and reject malformed vectors. */
export function normaliseBase(base, where = '[base]', errors = []) {
  const value = base && typeof base === 'object' && !Array.isArray(base) ? base : {};
  if (base !== undefined && base !== null && value !== base) errors.push(`${where} must be a table`);
  return {
    offset: normaliseVector(value.offset, IDENTITY_BASE.offset, `${where}.offset`, errors),
    rotation: normaliseVector(value.rotation, IDENTITY_BASE.rotation, `${where}.rotation`, errors),
    scale: normaliseVector(value.scale, IDENTITY_BASE.scale, `${where}.scale`, errors),
  };
}

/** Stable payload hashed into each manifest record. */
export function canonicalBase(base) {
  const clean = normaliseBase(base);
  return [
    `offset=${clean.offset.join(',')}`,
    `rotation=${clean.rotation.join(',')}`,
    `scale=${clean.scale.join(',')}`,
  ].join(' ');
}

/** Every capture field is named so removing one is itself a recipe change. */
export function canonicalCaptureParams(params) {
  return [
    `yaw_views=${show(params.yawViews)}`,
    `resolution=${show(params.resolution)}`,
    `pitch=${show(params.pitch)}`,
  ].join(' ');
}

function captureBase(source, sidecarByPath, errors) {
  const baseSource = source.replace(/\.glb$/i, '.model.toml');
  const sidecar = sidecarByPath.get(baseSource);
  if (!sidecar) {
    const base = normaliseBase(IDENTITY_BASE);
    return { base, baseSource: 'identity', baseSha256: sha256(canonicalBase(base)) };
  }
  const base = normaliseBase(sidecar.doc?.base, `${baseSource} [base]`, errors);
  return { base, baseSource, baseSha256: sha256(canonicalBase(base)) };
}

function captureParams(capture, where, errors) {
  const params = {
    yawViews: capture?.yaw_views ?? null,
    resolution: capture?.resolution ?? null,
    pitch: capture?.pitch ?? null,
  };
  if (!(Number.isInteger(params.yawViews) && params.yawViews > 0)) {
    errors.push(`${where}.yaw_views must be a positive whole number`);
  }
  if (!(Number.isInteger(params.resolution) && params.resolution > 0)) {
    errors.push(`${where}.resolution must be a positive whole number`);
  }
  if (!Number.isFinite(params.pitch)) errors.push(`${where}.pitch must be a finite number`);
  return params;
}

/**
 * Pure collection of all `[lod.capture]` outputs from parsed rig sidecars.
 *
 * Sidecars may share an output only when the capture source, authored recipe,
 * and source-model base transform agree. `[[lod]].scale` is deliberately not
 * compared: asteroid variants share one identity-captured atlas but size their
 * quads differently at runtime.
 */
export function collectCaptureTargets(sidecars) {
  const errors = [];
  const byOutput = new Map();
  const ordered = sidecars
    .map((sidecar) => ({
      ...sidecar,
      path: canonicalRepoPath(sidecar.path, 'sidecar path', errors) ?? sidecar.path,
    }))
    .sort((a, b) => a.path.localeCompare(b.path));
  const sidecarByPath = new Map(ordered.map((sidecar) => [sidecar.path, sidecar]));

  for (const { path: sidecarPath, doc } of ordered) {
    const levels = Array.isArray(doc?.lod) ? doc.lod : [];
    levels.forEach((level, index) => {
      if (!level?.capture) return;
      const where = `${sidecarPath} [[lod]] #${index} [lod.capture]`;
      const output = canonicalRepoPath(level.billboard, `${where}.billboard`, errors);
      if (!output) {
        errors.push(`${where}: names no billboard PNG output`);
        return;
      }
      if (!/\.png$/i.test(output)) errors.push(`${where}.billboard must name a .png file`);
      const source = canonicalRepoPath(level.capture.source, `${where}.source`, errors);
      if (!source || !/\.glb$/i.test(source)) {
        errors.push(`${where}.source must name a .glb file`);
        return;
      }
      const params = captureParams(level.capture, where, errors);
      const { base, baseSource, baseSha256 } = captureBase(source, sidecarByPath, errors);
      const candidate = {
        output,
        source,
        params,
        base,
        baseSource,
        baseSha256,
        recipeVersion: CAPTURE_RECIPE_VERSION,
        declaredBy: [sidecarPath],
      };
      const existing = byOutput.get(output);
      if (!existing) {
        byOutput.set(output, candidate);
        return;
      }
      if (
        existing.source !== source ||
        canonicalCaptureParams(existing.params) !== canonicalCaptureParams(params) ||
        existing.baseSource !== baseSource ||
        canonicalBase(existing.base) !== canonicalBase(base)
      ) {
        errors.push(
          `${output}: declared differently by ${existing.declaredBy[0]} and ${sidecarPath} - ` +
            'sidecars sharing one atlas must agree on source, capture parameters, and source base transform',
        );
      } else if (!existing.declaredBy.includes(sidecarPath)) {
        existing.declaredBy.push(sidecarPath);
      }
    });
  }

  const targets = [...byOutput.values()].sort((a, b) => a.output.localeCompare(b.output));
  for (const target of targets) target.declaredBy.sort();
  return { targets, errors };
}

/** Positional filters match an output, source, or declaring sidecar. */
export function matchesCaptureFilter(target, filters) {
  if (!filters.length) return true;
  const haystack = [target.output, target.source, ...target.declaredBy].join(' ');
  return filters.some((filter) => haystack.includes(filter));
}

/** Deterministic native-render invocation for one authored capture target. */
export function captureCommand(target, bin = BIN) {
  return {
    file: bin,
    args: [
      target.source,
      target.output,
      '--views',
      String(target.params.yawViews),
      '--resolution',
      String(target.params.resolution),
      '--pitch',
      String(target.params.pitch),
    ],
  };
}

function tomlNumber(value) {
  return Number.isInteger(value) ? `${value}.0` : String(value);
}

function paramsTable(params) {
  return `{ yaw_views = ${params.yawViews}, resolution = ${params.resolution}, pitch = ${tomlNumber(params.pitch)} }`;
}

function vector(value) {
  return `[${value.map(tomlNumber).join(', ')}]`;
}

function baseTable(base) {
  return `{ offset = ${vector(base.offset)}, rotation = ${vector(base.rotation)}, scale = ${vector(base.scale)} }`;
}

/** Construct one manifest record from a declaration and hashes observed on disk. */
export function captureManifestEntry(target, observed) {
  return {
    path: target.output,
    source: target.source,
    source_sha256: observed.sourceSha256,
    params: canonicalCaptureParams(target.params),
    paramsValues: { ...target.params },
    recipe_version: target.recipeVersion,
    base_source: target.baseSource,
    base: normaliseBase(target.base),
    base_sha256: target.baseSha256,
    output_sha256: observed.outputSha256,
    output_bytes: observed.outputBytes,
    declared_by: [...target.declaredBy],
  };
}

/** Stable, reviewable TOML rendering for capture provenance. */
export function formatCaptureManifest(entries) {
  const lines = [
    '# Generated by scripts/capture-billboards.mjs (issue #1245) - do not edit by hand.',
    '#',
    '# One record per sidecar-owned [lod.capture] PNG. The currency check hashes',
    '# source and output bytes and compares the authored recipe plus the exact',
    '# source-model [base] transform applied by the native capture tool.',
    '',
    `version = ${CAPTURE_MANIFEST_VERSION}`,
  ];
  for (const entry of [...entries].sort((a, b) => a.path.localeCompare(b.path))) {
    lines.push('');
    lines.push('[[output]]');
    lines.push(`path = ${JSON.stringify(entry.path)}`);
    lines.push(`source = ${JSON.stringify(entry.source)}`);
    lines.push(`source_sha256 = ${JSON.stringify(entry.source_sha256 ?? '')}`);
    lines.push(`params = ${paramsTable(entry.paramsValues)}`);
    lines.push(`recipe_version = ${entry.recipe_version}`);
    lines.push(`base_source = ${JSON.stringify(entry.base_source)}`);
    lines.push(`base = ${baseTable(entry.base)}`);
    lines.push(`base_sha256 = ${JSON.stringify(entry.base_sha256 ?? '')}`);
    lines.push(`output_sha256 = ${JSON.stringify(entry.output_sha256 ?? '')}`);
    lines.push(`output_bytes = ${entry.output_bytes ?? 0}`);
    lines.push(`declared_by = [${entry.declared_by.map((item) => JSON.stringify(item)).join(', ')}]`);
  }
  return `${lines.join('\n')}\n`;
}

function requireString(value, field) {
  if (typeof value !== 'string' || !value.length) throw new Error(`${field} must be a non-empty string`);
  return value;
}

function requireHash(value, field) {
  if (typeof value !== 'string' || !/^[a-f0-9]{64}$/.test(value)) {
    throw new Error(`${field} must be a lowercase SHA-256 digest`);
  }
  return value;
}

/** Parse and validate the committed manifest back into record-shaped values. */
export function parseCaptureManifest(text) {
  const doc = parseToml(text);
  if (doc.version !== CAPTURE_MANIFEST_VERSION) {
    throw new Error(`unsupported capture manifest version ${doc.version ?? 'missing'}`);
  }
  if (doc.output !== undefined && !Array.isArray(doc.output)) {
    throw new Error('capture manifest output must be an array of tables');
  }
  const outputs = doc.output ?? [];
  const parsed = outputs.map((output, index) => {
    const at = `output #${index}`;
    const params = {
      yawViews: output.params?.yaw_views ?? null,
      resolution: output.params?.resolution ?? null,
      pitch: output.params?.pitch ?? null,
    };
    const paramErrors = [];
    captureParams(
      { yaw_views: params.yawViews, resolution: params.resolution, pitch: params.pitch },
      `${at}.params`,
      paramErrors,
    );
    if (paramErrors.length) throw new Error(paramErrors.join('; '));
    const baseErrors = [];
    const base = normaliseBase(output.base, `${at}.base`, baseErrors);
    if (baseErrors.length) throw new Error(baseErrors.join('; '));
    const baseSha256 = requireHash(output.base_sha256, `${at}.base_sha256`);
    if (baseSha256 !== sha256(canonicalBase(base))) {
      throw new Error(`${at}.base_sha256 does not match its canonical base transform`);
    }
    if (!Number.isInteger(output.output_bytes) || output.output_bytes < 0) {
      throw new Error(`${at}.output_bytes must be a non-negative whole number`);
    }
    if (!Number.isInteger(output.recipe_version) || output.recipe_version <= 0) {
      throw new Error(`${at}.recipe_version must be a positive whole number`);
    }
    if (
      !Array.isArray(output.declared_by) ||
      !output.declared_by.length ||
      output.declared_by.some((item) => typeof item !== 'string')
    ) {
      throw new Error(`${at}.declared_by must be an array of paths`);
    }
    const declaredBy = output.declared_by.map((item, declarationIndex) => {
      const declarationErrors = [];
      const canonical = canonicalRepoPath(
        item,
        `${at}.declared_by #${declarationIndex}`,
        declarationErrors,
      );
      if (declarationErrors.length) throw new Error(declarationErrors.join('; '));
      return canonical;
    });
    let baseSource = requireString(output.base_source, `${at}.base_source`);
    if (baseSource !== 'identity') {
      const baseSourceErrors = [];
      baseSource = canonicalRepoPath(baseSource, `${at}.base_source`, baseSourceErrors);
      if (baseSourceErrors.length) throw new Error(baseSourceErrors.join('; '));
      if (!/\.model\.toml$/i.test(baseSource)) throw new Error(`${at}.base_source must name a .model.toml file or identity`);
    }
    const outputPath = canonicalRepoPath(requireString(output.path, `${at}.path`), `${at}.path`, []) ?? '';
    const sourcePath = canonicalRepoPath(requireString(output.source, `${at}.source`), `${at}.source`, []) ?? '';
    if (outputPath && !/\.png$/i.test(outputPath)) throw new Error(`${at}.path must name a .png file`);
    if (sourcePath && !/\.glb$/i.test(sourcePath)) throw new Error(`${at}.source must name a .glb file`);
    if (new Set(declaredBy).size !== declaredBy.length) {
      throw new Error(`${at}.declared_by contains duplicate paths`);
    }
    return {
      path: outputPath,
      source: sourcePath,
      source_sha256: requireHash(output.source_sha256, `${at}.source_sha256`),
      params: canonicalCaptureParams(params),
      paramsValues: params,
      recipe_version: output.recipe_version,
      base_source: baseSource,
      base,
      base_sha256: baseSha256,
      output_sha256: requireHash(output.output_sha256, `${at}.output_sha256`),
      output_bytes: output.output_bytes,
      declared_by: declaredBy.sort(),
    };
  });
  const paths = new Set();
  for (const entry of parsed) {
    if (!entry.path || !entry.source) throw new Error('capture manifest paths must be repo-relative');
    if (paths.has(entry.path)) throw new Error(`duplicate capture manifest output ${entry.path}`);
    paths.add(entry.path);
  }
  return parsed;
}

/** Compare recorded provenance with current declarations and file hashes. */
export function compareCaptureManifest(entries, observed) {
  const findings = [];
  const recorded = new Map(entries.map((entry) => [entry.path, entry]));
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
      findings.push({ output: target.output, kind: 'missing-output', detail: 'the billboard PNG is not on disk' });
      continue;
    }
    if (item.sourceSha256 === null) {
      findings.push({ output: target.output, kind: 'missing-source', detail: `${target.source} is not on disk` });
      continue;
    }
    if (entry.source !== target.source) {
      findings.push({
        output: target.output,
        kind: 'source-repointed',
        detail: `captured from ${entry.source}, sidecar now says ${target.source}`,
      });
    } else if (entry.source_sha256 !== item.sourceSha256) {
      findings.push({
        output: target.output,
        kind: 'source-changed',
        detail: `${target.source} changed since this atlas was captured`,
      });
    }
    const params = canonicalCaptureParams(target.params);
    if (entry.params !== params) {
      findings.push({
        output: target.output,
        kind: 'params-changed',
        detail: `captured with [${entry.params}], sidecar now says [${params}]`,
      });
    }
    if (entry.recipe_version !== target.recipeVersion) {
      findings.push({
        output: target.output,
        kind: 'recipe-changed',
        detail: `captured with recipe ${entry.recipe_version}, tool now requires ${target.recipeVersion}`,
      });
    }
    if (entry.base_source !== target.baseSource || entry.base_sha256 !== target.baseSha256) {
      findings.push({
        output: target.output,
        kind: 'base-changed',
        detail: `capture base changed from ${entry.base_source} to ${target.baseSource}`,
      });
    }
    if (entry.output_sha256 !== item.outputSha256 || entry.output_bytes !== item.outputBytes) {
      findings.push({
        output: target.output,
        kind: 'output-changed',
        detail: 'the billboard PNG on disk is not the one that was recorded',
      });
    }
    const expectedDimensions = [target.params.yawViews * target.params.resolution, target.params.resolution];
    const hasDimensions = Array.isArray(item.outputDimensions) && item.outputDimensions.length === 2;
    if (
      !hasDimensions ||
      item.outputDimensions[0] !== expectedDimensions[0] ||
      item.outputDimensions[1] !== expectedDimensions[1]
    ) {
      findings.push({
        output: target.output,
        kind: 'dimensions-changed',
        detail: !hasDimensions
          ? 'output is not a valid PNG with an IHDR chunk'
          : `PNG is ${item.outputDimensions.join('x')}, recipe requires ${expectedDimensions.join('x')}`,
      });
    }
    const declaredBy = [...target.declaredBy].sort();
    if (entry.declared_by.join('\n') !== declaredBy.join('\n')) {
      findings.push({
        output: target.output,
        kind: 'declarations-changed',
        detail: 'the set of sidecars sharing this atlas changed',
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

export function describeCaptureFindings(findings) {
  return findings.map((finding) => `  ${finding.output}: ${finding.kind} - ${finding.detail}`).join('\n');
}

/**
 * Pure filtered update: retain untouched declared records, replace observed
 * ones, and prune only records whose output is absent from the full target set.
 */
export function updateCaptureManifestEntries(allTargets, recorded, observed) {
  const declared = new Set(allTargets.map((target) => target.output));
  const entries = new Map(
    recorded.filter((entry) => declared.has(entry.path)).map((entry) => [entry.path, entry]),
  );
  for (const item of observed) entries.set(item.target.output, captureManifestEntry(item.target, item));
  return [...entries.values()].sort((a, b) => a.path.localeCompare(b.path));
}

/** Read and parse every model rig sidecar at the filesystem edge. */
export async function readCaptureSidecars(root = process.cwd()) {
  const dir = path.join(root, MODELS_DIR);
  const files = (await readdir(dir)).filter((file) => file.endsWith('.toml')).sort();
  const sidecars = [];
  for (const file of files) {
    const sidecarPath = `${MODELS_DIR}/${file}`;
    const text = await readFile(path.join(root, sidecarPath), 'utf8');
    try {
      sidecars.push({ path: sidecarPath, doc: parseToml(text) });
    } catch (error) {
      throw new Error(`${sidecarPath}: ${error.message}`);
    }
  }
  return sidecars;
}

function hashFile(file) {
  if (!existsSync(file)) return null;
  return sha256(readFileSync(file));
}

/** Read only the PNG signature + IHDR dimensions; null means not a valid PNG. */
export function pngDimensions(bytes) {
  if (!Buffer.isBuffer(bytes)) bytes = Buffer.from(bytes);
  if (bytes.length < 24 || bytes.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a') return null;
  if (bytes.subarray(12, 16).toString('ascii') !== 'IHDR') return null;
  return [bytes.readUInt32BE(16), bytes.readUInt32BE(20)];
}

/** Hash selected capture inputs/outputs without invoking the renderer. */
export function observeCaptureTargets(root, targets) {
  return targets.map((target) => {
    const outputFile = path.join(root, target.output);
    const outputBytes = existsSync(outputFile) ? readFileSync(outputFile) : null;
    return {
      target,
      sourceSha256: hashFile(path.join(root, target.source)),
      outputSha256: outputBytes === null ? null : sha256(outputBytes),
      outputBytes: outputBytes === null ? null : statSync(outputFile).size,
      outputDimensions: outputBytes === null ? null : pngDimensions(outputBytes),
    };
  });
}

async function recordedManifest(root) {
  const file = path.join(root, CAPTURE_MANIFEST_PATH);
  return existsSync(file) ? parseCaptureManifest(await readFile(file, 'utf8')) : [];
}

async function writeCaptureManifest(root, allTargets, recorded, observed) {
  const entries = updateCaptureManifestEntries(allTargets, recorded, observed);
  await writeFile(path.join(root, CAPTURE_MANIFEST_PATH), formatCaptureManifest(entries));
  return entries;
}

/**
 * Re-hash and update selected manifest records. Used by `--adopt` and by the
 * model viewer after it has successfully written an atlas and its sidecars.
 */
export async function refreshCaptureManifest(root = process.cwd(), filters = []) {
  const { targets: allTargets, errors } = collectCaptureTargets(await readCaptureSidecars(root));
  if (errors.length) throw new Error(errors.join('\n'));
  const selected = allTargets.filter((target) => matchesCaptureFilter(target, filters));
  if (!selected.length) throw new Error(`no declared capture output matches ${filters.join(' ')}`);
  const observed = observeCaptureTargets(root, selected);
  const missing = observed.filter((item) => item.sourceSha256 === null || item.outputSha256 === null);
  if (missing.length) {
    throw new Error(`cannot record missing capture files: ${missing.map((item) => item.target.output).join(', ')}`);
  }
  const malformed = observed.filter((item) => {
    const expected = [item.target.params.yawViews * item.target.params.resolution, item.target.params.resolution];
    return item.outputDimensions === null || item.outputDimensions[0] !== expected[0] || item.outputDimensions[1] !== expected[1];
  });
  if (malformed.length) {
    throw new Error(`cannot record PNGs whose IHDR does not match the recipe: ${malformed.map((item) => item.target.output).join(', ')}`);
  }
  await writeCaptureManifest(root, allTargets, await recordedManifest(root), observed);
  return observed.map((item) => item.target.output);
}

/** Update billboard dimensions while keeping the authored capture recipe. */
async function attachCapturedBillboard(root, target, meta) {
  for (const sidecarPath of target.declaredBy) {
    const absolute = path.join(root, sidecarPath);
    const text = await readFile(absolute, 'utf8');
    const doc = parseToml(text);
    const levels = ladderFromDoc(doc);
    const index = levels.findIndex((level) => level.billboard === target.output && level.capture);
    if (index < 0) throw new Error(`${sidecarPath} no longer declares ${target.output}`);
    // The native tool applies <source>.model.toml. When that sidecar is absent
    // it captures identity geometry (asteroids), so the declaring variant's
    // uniform base scale is applied to the quad instead.
    const variantScale = target.baseSource === 'identity' ? Number(doc?.base?.scale?.[0] ?? 1) : 1;
    levels[index] = {
      ...levels[index],
      scale: [round(meta.world_w * variantScale), round(meta.world_h * variantScale), 1],
      capture: {
        source: target.source,
        yaw_views: target.params.yawViews,
        resolution: target.params.resolution,
        pitch: target.params.pitch,
      },
    };
    const next = replaceLadder(text, levels);
    if (next !== text) await writeFile(absolute, next);
  }
}

function runCapture(target) {
  const command = captureCommand(target);
  const stdout = execFileSync(command.file, command.args, {
    encoding: 'utf8',
    maxBuffer: 16 * 1024 * 1024,
  });
  const line = stdout.trim().split('\n').filter(Boolean).pop() ?? '{}';
  const meta = JSON.parse(line);
  if (!Number.isFinite(meta.world_w) || !Number.isFinite(meta.world_h)) {
    throw new Error(`capture tool returned no world dimensions for ${target.output}`);
  }
  return meta;
}

function parseArgs(argv) {
  const flags = new Set(argv.filter((arg) => arg.startsWith('--')));
  return {
    check: flags.has('--check'),
    adopt: flags.has('--adopt'),
    filters: argv.filter((arg) => !arg.startsWith('--')),
    unknown: [...flags].filter((flag) => !['--check', '--adopt'].includes(flag)),
  };
}

async function main() {
  const root = process.cwd();
  const cli = parseArgs(process.argv.slice(2));
  if (cli.unknown.length || (cli.check && cli.adopt)) {
    console.error(
      cli.unknown.length
        ? `[capture-billboards] unknown flag(s): ${cli.unknown.join(', ')}`
        : '[capture-billboards] --check and --adopt are mutually exclusive',
    );
    process.exit(2);
  }

  const { targets: allTargets, errors } = collectCaptureTargets(await readCaptureSidecars(root));
  if (errors.length) {
    console.error('[capture-billboards] invalid or conflicting [lod.capture] declarations:');
    for (const error of errors) console.error(`  ${error}`);
    process.exit(1);
  }
  const selected = allTargets.filter((target) => matchesCaptureFilter(target, cli.filters));
  if (!selected.length) {
    console.error(`[capture-billboards] no declared capture output matches ${cli.filters.join(' ')}`);
    process.exit(1);
  }
  const recorded = await recordedManifest(root);

  if (cli.check) {
    const selectedPaths = new Set(selected.map((target) => target.output));
    const relevantRecorded = cli.filters.length
      ? recorded.filter((entry) => selectedPaths.has(entry.path))
      : recorded;
    const findings = compareCaptureManifest(relevantRecorded, observeCaptureTargets(root, selected));
    if (findings.length) {
      console.error(`[capture-billboards] ${findings.length} billboard capture finding(s):`);
      console.error(describeCaptureFindings(findings));
      console.error(
        `\n  Recapture with \`node scripts/capture-billboards.mjs <model>\`, or deliberately\n` +
          `  adopt reviewed committed PNGs with \`node scripts/capture-billboards.mjs --adopt <model>\`.\n` +
          `  Commit the PNG, sidecar, and ${CAPTURE_MANIFEST_PATH} together.`,
      );
      process.exit(1);
    }
    console.error(`[capture-billboards] ${selected.length} capture output(s) up to date`);
    return;
  }

  if (cli.adopt) {
    await refreshCaptureManifest(root, cli.filters);
    console.error(`[capture-billboards] adopted ${selected.length} committed output(s) into ${CAPTURE_MANIFEST_PATH}`);
    return;
  }

  if (!existsSync(path.join(root, BIN))) {
    console.error(
      `[capture-billboards] ${BIN} not found - build it first:\n` +
        '  cargo build --features capture --bin capture-billboard',
    );
    process.exit(1);
  }
  for (const target of selected) {
    process.stderr.write(`[capture-billboards] ${target.output}... `);
    const meta = runCapture(target);
    await attachCapturedBillboard(root, target, meta);
    console.error(`${meta.world_w.toFixed(2)}x${meta.world_h.toFixed(2)}`);
  }
  const observed = observeCaptureTargets(root, selected);
  await writeCaptureManifest(root, allTargets, recorded, observed);
  console.error(`[capture-billboards] updated ${CAPTURE_MANIFEST_PATH}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(`[capture-billboards] ${error.message}`);
    process.exit(1);
  });
}
