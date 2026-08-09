// viewer-lods.mjs — reading and rewriting a model's `[[lod]]` ladder.
//
// The pure half of the model viewer's LOD panel: everything here is a function
// over strings and plain objects, so the authoring rules are unit-tested
// (tests/client/viewer-lods.test.js) without a server, a browser or a file.
// scripts/dev-viewer.mjs is the only caller that touches disk.
//
// ── Why the viewer writes sidecars at all ───────────────────────────────────
//
// The ladder is authored in the rig sidecar and nowhere else (issue #914), and
// the decimation parameters that produce a generated level live beside it in
// `[lod.generate]` (issue #919). Judging a decimated hull is an *eyeball*
// operation — a silhouette that lost its turrets is not something the growth
// gate can see — so the loop "look at LOD2, drop the ratio, look again" wants
// the ladder editable from the same page that renders it. Writing the sidecar
// keeps the single authoring location: the panel edits the same file an editor
// would, and `node scripts/generate-lods.mjs` remains the only thing that makes
// a .glb.
//
// ── Variants move together ──────────────────────────────────────────────────
//
// One model can have several rig sidecars (`<stem>.small.toml`,
// `<stem>.large.toml`, `<stem>.huge.toml`, `<stem>.cosmetic.toml`) and they share one generated
// file per level. `collectTargets` in generate-lods.mjs rejects a tree where
// two sidecars claim the same output with different parameters, so an edit
// applies to *every* sidecar of the stem rather than to the one the viewer
// happens to be showing. Saving one variant and silently leaving the others
// behind would produce exactly the disagreement that check refuses to run with.

import { parse as parseToml } from 'smol-toml';
import { collectTargets, tomlFloat } from './generate-lods.mjs';

/** Keys a `[[lod]]` level may carry, in the order they are written back out. */
const LEVEL_KEYS = [
  'max_distance',
  'model',
  'variant',
  'billboard',
  'shape',
  'colour',
  'radius',
  'size',
  'minor_radius',
  'emissive',
  'scale',
  'rotation',
];

/** Keys of the `[lod.generate]` sub-table, in written order. */
const GENERATE_KEYS = ['source', 'ratio', 'error', 'texture_size', 'remesh_voxel_size'];

/** Keys of the `[lod.capture]` sub-table, in written order. */
const CAPTURE_KEYS = ['source', 'yaw_views', 'resolution', 'pitch'];

/** Pixel counts / view counts are integers; every other generate/capture key is a float. */
const INTEGER_KEYS = new Set(['texture_size', 'yaw_views', 'resolution']);

/**
 * The `<stem>` of a model path: `assets/models/x.glb` → `x`.
 * Anything without the extension is returned as its own basename.
 */
export function modelStem(modelPath) {
  const base = String(modelPath).split('/').pop() ?? '';
  return base.toLowerCase().endsWith('.glb') ? base.slice(0, -'.glb'.length) : base;
}

/**
 * Which of `files` are rig sidecars for `stem`.
 *
 * `<stem>.model.toml` is the base rig and `<stem>.<variant>.toml` is a variant;
 * both are sidecars of the same model and both carry the ladder. The match is
 * on the whole stem, so `asteroid_common_1` never picks up
 * `asteroid_common_1_lod1.large.toml` — that file is the *generated level's*
 * own rig, which has no ladder of its own.
 */
export function sidecarsForStem(files, stem) {
  return files
    .filter((f) => {
      if (!f.endsWith('.toml')) return false;
      const withoutExt = f.slice(0, -'.toml'.length);
      const dot = withoutExt.lastIndexOf('.');
      return dot !== -1 && withoutExt.slice(0, dot) === stem;
    })
    .sort();
}

/**
 * The ladder a parsed sidecar declares, as plain level objects.
 *
 * Missing keys stay missing rather than becoming nulls: the sidecar schema
 * treats an omitted field as "inherit from the entity's flat `[mesh]`", and a
 * round trip through the panel must not turn an inherited colour into an
 * explicitly authored one.
 */
export function ladderFromDoc(doc) {
  const levels = Array.isArray(doc?.lod) ? doc.lod : [];
  return levels.map((level) => {
    const out = {};
    for (const key of LEVEL_KEYS) {
      if (level?.[key] !== undefined) out[key] = level[key];
    }
    if (level?.generate) {
      const generate = {};
      for (const key of GENERATE_KEYS) {
        if (level.generate[key] !== undefined) generate[key] = level.generate[key];
      }
      out.generate = generate;
    }
    if (level?.capture) {
      const capture = {};
      for (const key of CAPTURE_KEYS) {
        if (level.capture[key] !== undefined) capture[key] = level.capture[key];
      }
      out.capture = capture;
    }
    return out;
  });
}

/** A TOML scalar: strings quoted, integers bare, every other number a float. */
function tomlValue(key, value) {
  if (typeof value === 'string') return JSON.stringify(value);
  if (typeof value === 'boolean') return String(value);
  if (Array.isArray(value)) {
    return `[ ${value.map((v) => tomlValue(key, v)).join(', ')} ]`;
  }
  return INTEGER_KEYS.has(key) ? String(Math.round(value)) : tomlFloat(value);
}

/**
 * Render a ladder as the `[[lod]]` blocks of a sidecar.
 *
 * Deterministic in key order and spacing so re-saving an unedited ladder
 * produces a byte-identical file — an edit that reshuffles keys would show up
 * as a diff on every model the panel ever opened.
 */
export function renderLadder(levels) {
  const blocks = levels.map((level) => {
    const lines = ['[[lod]]'];
    for (const key of LEVEL_KEYS) {
      if (level[key] === undefined || level[key] === null) continue;
      lines.push(`${key} = ${tomlValue(key, level[key])}`);
    }
    const generate = level.generate;
    if (generate && Object.keys(generate).length) {
      lines.push('', '[lod.generate]');
      for (const key of GENERATE_KEYS) {
        if (generate[key] === undefined || generate[key] === null) continue;
        lines.push(`${key} = ${tomlValue(key, generate[key])}`);
      }
    }
    const capture = level.capture;
    if (capture && Object.keys(capture).length) {
      lines.push('', '[lod.capture]');
      for (const key of CAPTURE_KEYS) {
        if (capture[key] === undefined || capture[key] === null) continue;
        lines.push(`${key} = ${tomlValue(key, capture[key])}`);
      }
    }
    return lines.join('\n');
  });
  return blocks.join('\n\n');
}

/**
 * Split a sidecar into the text before its ladder, the comment block that
 * introduces the ladder, and the text after it.
 *
 * Sections are found by their header lines rather than by re-serialising the
 * parsed document: everything outside the ladder — `[base]`, `[extents]`, every
 * `[markers.*]`, and the comments explaining them — has to survive a save
 * untouched, and a TOML writer would reformat all of it.
 *
 * The comment block immediately above the first `[[lod]]` is carried across
 * rather than dropped. On the shipped asteroids that is a paragraph explaining
 * what the ladder is for and how to regenerate it; a panel that quietly deleted
 * it the first time someone nudged a ratio would be worse than one that
 * refused to save.
 */
export function splitLadder(text) {
  const lines = text.split('\n');
  const isHeader = (line) => /^\s*\[\[?[^\]]+\]\]?\s*$/.test(line);
  const headerKey = (line) => line.trim().replace(/^\[+/, '').replace(/\]+$/, '').trim();
  const isLadder = (line) => {
    const key = headerKey(line);
    return key === 'lod' || key.startsWith('lod.');
  };

  const before = [];
  const after = [];
  let comment = [];
  // `pending` holds the run of comment/blank lines since the last content line.
  // It belongs to whatever comes next — a section header, another line, or the
  // ladder, whose introduction is the one comment block carried across.
  let pending = [];
  let seenLadder = false;
  let dropping = false;

  for (const line of lines) {
    const blankOrComment = line.trim() === '' || line.trim().startsWith('#');

    if (isHeader(line)) {
      if (isLadder(line)) {
        if (!seenLadder) {
          // The first ladder header claims the comment block above it.
          comment = trimBlankEdges(pending);
          seenLadder = true;
        }
        pending = [];
        dropping = true;
        continue;
      }
      (seenLadder ? after : before).push(...pending, line);
      pending = [];
      dropping = false;
      continue;
    }
    if (blankOrComment) {
      // Held back: inside the ladder these introduce whatever section follows,
      // and outside it they introduce the next line.
      pending.push(line);
      continue;
    }
    // A key/value line inside the ladder is what this function drops.
    if (dropping) continue;
    (seenLadder ? after : before).push(...pending, line);
    pending = [];
  }
  // Trailing comments after the last content line survive, unless the ladder
  // was the last thing in the file — then they are the ladder's own trailing
  // notes and go with it.
  if (!dropping) (seenLadder ? after : before).push(...pending);

  return {
    before: trimTrailingBlanks(before).join('\n'),
    comment: comment.join('\n'),
    after: trimBlankEdges(after).join('\n'),
    hadLadder: seenLadder,
  };
}

function trimTrailingBlanks(lines) {
  const out = [...lines];
  while (out.length && out[out.length - 1].trim() === '') out.pop();
  return out;
}

function trimBlankEdges(lines) {
  const out = trimTrailingBlanks(lines);
  while (out.length && out[0].trim() === '') out.shift();
  return out;
}

/**
 * A sidecar's text with its ladder replaced by `levels`.
 *
 * An empty ladder removes the `[[lod]]` blocks entirely (and the comment that
 * introduced them, which describes a ladder that no longer exists). Sections
 * that followed the ladder keep their place after it.
 */
export function replaceLadder(text, levels) {
  const { before, comment, after } = splitLadder(text);
  const parts = [];
  if (before.trim()) parts.push(before);
  if (levels.length) {
    // The comment sits directly on top of the first `[[lod]]`, as an author
    // would write it — one part, not two separated by a blank line.
    const ladder = renderLadder(levels);
    parts.push(comment ? `${comment}\n${ladder}` : ladder);
  }
  if (after.trim()) parts.push(after);
  return `${parts.join('\n\n')}\n`;
}

/**
 * Structural problems with a proposed ladder, as messages.
 *
 * This is the *shape* check — the things that make a ladder meaningless before
 * anything is generated. The parameter rules (`ratio` in range, variants
 * agreeing about a shared output) belong to the generator and are checked by
 * running its own `collectTargets` over the proposed documents; see
 * [`validateProposal`].
 */
export function validateLadder(levels) {
  const problems = [];
  if (!levels.length) return problems;

  levels.forEach((level, i) => {
    const where = `level ${i}`;
    if (!level.model && !level.shape) {
      problems.push(`${where}: needs either a model or a procedural shape`);
    }
    if (level.model && level.shape) {
      problems.push(`${where}: has both a model and a shape — a level is one or the other`);
    }
    if (level.generate && !level.model) {
      problems.push(`${where}: declares [lod.generate] but is not a generated GLB level`);
    }
    if (level.max_distance !== undefined && !(level.max_distance > 0)) {
      problems.push(`${where}: max_distance ${level.max_distance} must be greater than 0`);
    }
  });

  // Bands are read near→far, so an out-of-order distance silently shrinks a
  // level to nothing rather than failing anywhere.
  const bounded = levels.filter((l) => l.max_distance !== undefined);
  for (let i = 1; i < bounded.length; i += 1) {
    if (bounded[i].max_distance <= bounded[i - 1].max_distance) {
      problems.push(
        `max_distance must increase near→far: ${bounded[i - 1].max_distance} is followed by ${bounded[i].max_distance}`,
      );
    }
  }
  // Only the last level may be unbounded: an earlier one would swallow every
  // level after it.
  levels.forEach((level, i) => {
    if (level.max_distance === undefined && i !== levels.length - 1) {
      problems.push(`level ${i}: only the final level may omit max_distance`);
    }
  });

  return problems;
}

/**
 * Run the generator's own collector over a proposed set of sidecar texts.
 *
 * `proposed` is `[{ path, text }]` — every sidecar the save would write, not
 * just the edited one, so a disagreement introduced *between* variants is
 * caught here rather than by a red `npm run lods:check` three commits later.
 * Returns the generator's error messages, plus a parse error if the rendered
 * TOML does not read back.
 */
export function validateProposal(proposed) {
  const sidecars = [];
  for (const { path, text } of proposed) {
    try {
      sidecars.push({ path, doc: parseToml(text) });
    } catch (err) {
      return [`${path}: ${err.message}`];
    }
  }
  return collectTargets(sidecars).errors;
}

/**
 * The variant name a sidecar filename carries: `rock.large.toml` → `large`.
 *
 * `<stem>.model.toml` is the reserved default, and it reports as `""` — the
 * same empty value the viewer's variant dropdown uses for "(base)", so the two
 * can be compared without either side special-casing the name.
 */
export function variantOfSidecar(file, stem) {
  const variant = file.slice(stem.length + 1, -'.toml'.length);
  return variant === 'model' ? '' : variant;
}

/**
 * The largest dimension a sidecar's `[extents]` declares, or `null`.
 *
 * The one number that says how big a model is, and the thing switch distances
 * have to be proportional to: 50 m is far away for a courier and inside a
 * starbase.
 */
export function extentFromDoc(doc) {
  const size = doc?.extents?.size;
  if (!Array.isArray(size) || !size.length) return null;
  return size.reduce((a, b) => Math.max(a, Number(b)), 0) || null;
}

/**
 * The largest dimension in the model's OWN units — the raw GLB geometry, before
 * the rig's base scale is applied.
 *
 * The one number `remesh_voxel_size` is measured against, and the one the
 * sidecar does not otherwise show. Everything else in there — `max_distance`,
 * `[extents]` — is post-scale world units, so on a rock scaled 4.2× a voxel
 * size that looks small against an extent of 8 is in fact larger than half the
 * mesh. A voxel size of 1.0 on a 1.9-unit asteroid rebuilds it as a cube, which
 * is a slow way to learn which units the field is in.
 */
export function modelUnitExtent(doc) {
  const size = doc?.extents?.size;
  if (!Array.isArray(size) || !size.length) return null;
  const scale = doc?.base?.scale;
  const axis = (i) => {
    const s = Array.isArray(scale) ? Number(scale[i]) : Number(scale ?? 1);
    return Number(size[i]) / (Number.isFinite(s) && s !== 0 ? s : 1);
  };
  return size.reduce((a, _, i) => Math.max(a, axis(i)), 0) || null;
}

/**
 * Rebuild an existing ladder for another model, scaling its bands by size.
 *
 * The authoring pass ahead of this tool is ten hulls that have no ladder at
 * all, and the shipped asteroids already carry a ladder someone tuned. Copying
 * that shape — the same ratios, error limits and texture sizes, at distances
 * scaled by the new model's extents — starts from a decision that was made
 * rather than from numbers this file would have to invent (Key Constraint 11:
 * the precedent lives in the sidecars, not here).
 *
 * The result is a *proposal*. It lands in the panel's editor for the person to
 * change before anything is saved, which is the point: a hull is not a rock,
 * and the distances are exactly what wants an eye on it.
 */
export function templateLadder(template, target) {
  const scale =
    template.extent && target.extent ? target.extent / template.extent : 1;
  const rename = (p) =>
    typeof p === 'string' ? p.split(template.stem).join(target.stem) : p;

  return template.levels.map((level) => {
    const out = { ...level };
    if (out.max_distance !== undefined) {
      // One decimal is as precise as a switch distance ever needs to be, and
      // keeps the sidecar readable.
      out.max_distance = Math.round(out.max_distance * scale * 10) / 10;
    }
    if (out.model) out.model = rename(out.model);
    if (level.generate) {
      out.generate = { ...level.generate };
      if (out.generate.source) out.generate.source = rename(out.generate.source);
    }
    return out;
  });
}

/**
 * The generated outputs a ladder declares, in level order.
 *
 * The panel uses this to name the files a regeneration will rewrite, so the
 * button says what it is about to overwrite before it does.
 */
export function generatedOutputs(levels) {
  return levels.filter((l) => l.generate && l.model).map((l) => l.model);
}
