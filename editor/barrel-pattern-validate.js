// Shared authored barrel-pattern rules; adapters retain diagnostic vocabulary.
export function validateBarrelPatterns(rows, { path, label, duplicateLabel }) {
  const findings = [];
  if (!Array.isArray(rows)) return findings;

  const seenIds = new Set();

  for (let i = 0; i < rows.length; i++) {
    const row = rows[i];
    if (!row || typeof row !== 'object') continue;
    const basePath = `${path}[${i}]`;
    const id = row.id ?? i;

    // ── Duplicate id ──
    if (row.id !== undefined && row.id !== null) {
      if (seenIds.has(row.id)) {
        findings.push({
          path: `${basePath}.id`,
          severity: 'error',
          message: `Duplicate ${duplicateLabel} id "${row.id}"`,
        });
      }
      seenIds.add(row.id);
    }

    const barrels = Array.isArray(row.barrels) ? row.barrels : [];
    const barrelCount = barrels.length > 0 ? barrels.length : 1;
    const pattern = Array.isArray(row.pattern) ? row.pattern : [];

    // ── Empty pattern with multiple barrels ──
    if (pattern.length === 0) {
      if (barrels.length > 1) {
        findings.push({
          path: `${basePath}.pattern`,
          severity: 'error',
          message: `${label} "${id}" declares ${barrels.length} barrels but no firing pattern; multiple barrels require a pattern`,
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
          message: `${label} "${id}" pattern step ${s} is malformed`,
        });
        continue;
      }

      const stepBarrels = Array.isArray(step.barrels) ? step.barrels : null;
      if (!stepBarrels || stepBarrels.length === 0) {
        findings.push({
          path: `${stepPath}.barrels`,
          severity: 'error',
          message: `${label} "${id}" pattern step ${s} fires no barrels`,
        });
      } else {
        for (const b of stepBarrels) {
          if (!Number.isInteger(b) || b < 0 || b >= barrelCount) {
            findings.push({
              path: `${stepPath}.barrels`,
              severity: 'error',
              message: `${label} "${id}" pattern step ${s} references barrel index ${b} but only ${barrelCount} barrel(s) are declared`,
            });
          }
        }
      }

      const offset = step.offset_secs;
      if (offset !== undefined && (typeof offset !== 'number' || !isFinite(offset) || offset < 0)) {
        findings.push({
          path: `${stepPath}.offset_secs`,
          severity: 'error',
          message: `${label} "${id}" pattern step ${s} has offset_secs=${offset} (must be a number >= 0)`,
        });
      }
    }
  }

  return findings;
}
