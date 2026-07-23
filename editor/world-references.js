/**
 * world-references.js — Pure cross-reference validator for world TOML.
 *
 * Delegates to `CrossReferenceIndex` for the actual scan of triggers,
 * trigger actions, comms templates, and comms responses, then emits one
 * `{ path, severity, message }` record per reference whose `targetName`
 * isn't a known entity in the same world.
 *
 * Designed to be called by `validation.js validateFile` for world files.
 *
 * Anchor references on `[[entity]] anchor = "..."` are NOT validated
 * here because the runtime resolves anchors during parsing; absence
 * surfaces as a parse error from the Rust side.
 *
 * `[[comms]].from` is allowed to be either an entity name or a
 * free-form display string, so it is not required to resolve.
 */

import { CrossReferenceIndex } from './cross-references.js';

/**
 * Validate every entity reference in `worldObj` against the entity name
 * set declared in the same world.  Returns one record per unresolved
 * reference.
 *
 * The `path` field uses `CrossReferenceIndex`'s recorded `context`
 * string so the resulting messages stay aligned with whatever sites the
 * index scans — extending the index automatically extends this
 * validator.
 *
 * @param {object} worldObj  Parsed world TOML object.
 * @returns {Array<{ path: string, severity: 'error'|'warning', message: string }>}
 */
export function validateWorldReferences(worldObj) {
  const results = [];
  if (!worldObj || typeof worldObj !== 'object') return results;

  const index = new CrossReferenceIndex();
  index.indexLayers([{ path: '', worldState: worldObj }]);

  // De-duplicate so the same dangling name referenced from two sites
  // doesn't fire twice with identical messages — but we DO want one
  // entry per (name, context) pair so the editor can show every call
  // site that needs fixing.
  const seen = new Set();

  for (const ref of index.allReferences()) {
    const { targetName, context } = ref;
    if (index.hasEntity(targetName)) continue;
    // `comms.from` is a free-form display string (e.g. "Starbase Alpha"
    // or "Pirate Raider Captain") and is not required to resolve to an
    // entity declared in the world.  The index still records it so the
    // editor UI can highlight known senders, but the validator skips it.
    if (context === 'comms.from') continue;
    const key = `${targetName}::${context}`;
    if (seen.has(key)) continue;
    seen.add(key);
    // Dangling entity cross-references are NON-BLOCKING warnings (issue #757),
    // matching the Rust composition validator (`src/world/validate.rs`), which
    // reports a bare unresolved reference as `Severity::Warning`: the name may
    // resolve to a runtime-spawned or engine-provided entity, or belong to a
    // world still being authored across several files. Keeping it a warning
    // holds the editor consistent with the host's atomic-activation gate.
    results.push({
      path: context,
      severity: 'warning',
      message: `Reference to unknown entity "${targetName}" (${context})`,
    });
  }

  return results;
}
