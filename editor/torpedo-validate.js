import { validateBarrelPatterns } from './barrel-pattern-validate.js';

/**
 * torpedo-validate.js
 *
 * Pure validation for `[[torpedoes.tubes]]` barrel patterns (issue #766). The
 * JS twin of `validate_torpedo_tubes`' barrel-pattern checks in
 * `src/entities/config.rs`: an author must not be able to save a tube whose
 * timed barrel pattern references a barrel that doesn't exist, fires no
 * barrels in a step, uses a negative offset, or omits a pattern while
 * declaring more than one barrel.
 *
 * The barrel pattern reuses the exact schema blasters wired in issue #765; the
 * pattern governs only which authored barrel each launched round leaves from,
 * never how many rounds fire (the magazine/tube/volley model stays
 * authoritative). This validator only guards the authored schema.
 *
 * Emits the standard editor finding shape `{ path, severity, message }` so a
 * finding blocks a save through `SaveFlow` (issue #757). No DOM, no TOML IO —
 * unit-testable in plain node.
 */

/**
 * Validate every torpedo tube's barrel pattern.
 *
 * @param {Array} tubes  Parsed `torpedoes.tubes` array.
 * @returns {Array<{path, severity, message}>}
 */
export function validateTorpedoTubes(tubes) {
  return validateBarrelPatterns(tubes, {
    path: 'torpedoes.tubes', label: 'Torpedo tube', duplicateLabel: 'torpedo tube',
  });
}
