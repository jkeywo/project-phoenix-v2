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
  const findings = [];
  if (!Array.isArray(tubes)) return findings;

  const seenIds = new Set();

  for (let i = 0; i < tubes.length; i++) {
    const tube = tubes[i];
    if (!tube || typeof tube !== 'object') continue;
    const basePath = `torpedoes.tubes[${i}]`;
    const id = tube.id ?? i;

    // ── Duplicate id ──
    if (tube.id !== undefined && tube.id !== null) {
      if (seenIds.has(tube.id)) {
        findings.push({
          path: `${basePath}.id`,
          severity: 'error',
          message: `Duplicate torpedo tube id "${tube.id}"`,
        });
      }
      seenIds.add(tube.id);
    }

    const barrels = Array.isArray(tube.barrels) ? tube.barrels : [];
    const barrelCount = barrels.length > 0 ? barrels.length : 1;
    const pattern = Array.isArray(tube.pattern) ? tube.pattern : [];

    // ── Empty pattern with multiple barrels ──
    if (pattern.length === 0) {
      if (barrels.length > 1) {
        findings.push({
          path: `${basePath}.pattern`,
          severity: 'error',
          message: `Torpedo tube "${id}" declares ${barrels.length} barrels but no firing pattern; multiple barrels require a pattern`,
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
          message: `Torpedo tube "${id}" pattern step ${s} is malformed`,
        });
        continue;
      }

      const stepBarrels = Array.isArray(step.barrels) ? step.barrels : null;
      if (!stepBarrels || stepBarrels.length === 0) {
        findings.push({
          path: `${stepPath}.barrels`,
          severity: 'error',
          message: `Torpedo tube "${id}" pattern step ${s} fires no barrels`,
        });
      } else {
        for (const b of stepBarrels) {
          if (!Number.isInteger(b) || b < 0 || b >= barrelCount) {
            findings.push({
              path: `${stepPath}.barrels`,
              severity: 'error',
              message: `Torpedo tube "${id}" pattern step ${s} references barrel index ${b} but only ${barrelCount} barrel(s) are declared`,
            });
          }
        }
      }

      const offset = step.offset_secs;
      if (offset !== undefined && (typeof offset !== 'number' || !isFinite(offset) || offset < 0)) {
        findings.push({
          path: `${stepPath}.offset_secs`,
          severity: 'error',
          message: `Torpedo tube "${id}" pattern step ${s} has offset_secs=${offset} (must be a number >= 0)`,
        });
      }
    }
  }

  return findings;
}
