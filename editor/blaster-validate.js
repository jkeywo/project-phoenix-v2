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
  const findings = [];
  if (!Array.isArray(banks)) return findings;

  const seenIds = new Set();

  for (let i = 0; i < banks.length; i++) {
    const bank = banks[i];
    if (!bank || typeof bank !== 'object') continue;
    const basePath = `weapons_console.blaster_banks[${i}]`;
    const id = bank.id ?? i;

    // ── Duplicate id ──
    if (bank.id !== undefined && bank.id !== null) {
      if (seenIds.has(bank.id)) {
        findings.push({
          path: `${basePath}.id`,
          severity: 'error',
          message: `Duplicate blaster bank id "${bank.id}"`,
        });
      }
      seenIds.add(bank.id);
    }

    const barrels = Array.isArray(bank.barrels) ? bank.barrels : [];
    const barrelCount = barrels.length > 0 ? barrels.length : 1;
    const pattern = Array.isArray(bank.pattern) ? bank.pattern : [];

    // ── Empty pattern with multiple barrels ──
    if (pattern.length === 0) {
      if (barrels.length > 1) {
        findings.push({
          path: `${basePath}.pattern`,
          severity: 'error',
          message: `Blaster bank "${id}" declares ${barrels.length} barrels but no firing pattern; multiple barrels require a pattern`,
        });
      }
      continue;
    }

    // ── Per-step validation ──
    for (let s = 0; s < pattern.length; s++) {
      const step = pattern[s];
      const stepPath = `${basePath}.pattern[${s}]`;
      if (!step || typeof step !== 'object') {
        findings.push({
          path: stepPath,
          severity: 'error',
          message: `Blaster bank "${id}" pattern step ${s} is malformed`,
        });
        continue;
      }

      const stepBarrels = Array.isArray(step.barrels) ? step.barrels : null;
      if (!stepBarrels || stepBarrels.length === 0) {
        findings.push({
          path: `${stepPath}.barrels`,
          severity: 'error',
          message: `Blaster bank "${id}" pattern step ${s} fires no barrels`,
        });
      } else {
        for (const b of stepBarrels) {
          if (!Number.isInteger(b) || b < 0 || b >= barrelCount) {
            findings.push({
              path: `${stepPath}.barrels`,
              severity: 'error',
              message: `Blaster bank "${id}" pattern step ${s} references barrel index ${b} but only ${barrelCount} barrel(s) are declared`,
            });
          }
        }
      }

      const offset = step.offset_secs;
      if (offset !== undefined && (typeof offset !== 'number' || !isFinite(offset) || offset < 0)) {
        findings.push({
          path: `${stepPath}.offset_secs`,
          severity: 'error',
          message: `Blaster bank "${id}" pattern step ${s} has offset_secs=${offset} (must be a number >= 0)`,
        });
      }
    }
  }

  return findings;
}
