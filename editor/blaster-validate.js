import { validateBarrelPatterns } from './barrel-pattern-validate.js';

/**
 * blaster-validate.js
 *
 * Pure validation for `[[weapons_console.blaster_banks]]` barrel patterns
 * (issue #765). The JS twin of `validate_blaster_banks` in
 * `src/entities/config.rs`: an author must not be able to save a bank whose
 * timed barrel pattern references a barrel that doesn't exist, fires no
 * barrels in a step, uses a negative offset, or omits a pattern while
 * declaring more than one barrel.
 *
 * Emits the standard editor finding shape `{ path, severity, message }` so a
 * finding blocks a save through `SaveFlow` (issue #757). No DOM, no TOML IO —
 * unit-testable in plain node.
 */

/**
 * Validate every blaster bank's barrel pattern.
 *
 * @param {Array} banks  Parsed `weapons_console.blaster_banks` array.
 * @returns {Array<{path, severity, message}>}
 */
export function validateBlasterBanks(banks) {
  return validateBarrelPatterns(banks, {
    path: 'weapons_console.blaster_banks', label: 'Blaster bank', duplicateLabel: 'blaster bank',
  });
}
