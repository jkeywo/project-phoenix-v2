/** Translate the host's one-shot save/restore outcome at its display edge. */
import { t } from './strings.js';

export function snapshotDiagnosticText(raw) {
  let outcome;
  try { outcome = JSON.parse(String(raw)); } catch (_) { /* older host or external detail */ }
  const params = outcome && typeof outcome.params === 'object' && outcome.params !== null
    ? outcome.params : {};
  switch (outcome?.kind) {
    case 'no_run': return t('server.snapshot.no_run');
    case 'restore_starting': return t('server.snapshot.restore_starting');
    case 'saved_at_tick': return t('server.snapshot.saved_at_tick', params);
    case 'write_failed': return t('server.snapshot.write_failed', params);
    case 'resumed_at_tick': return t('server.snapshot.resumed_at_tick', params);
    case 'no_state': return t('server.snapshot.no_state');
    case 'layer_failed': return t('server.snapshot.layer_failed', params);
    case 'not_ready': return t('server.snapshot.not_ready', params);
    case 'digest_mismatch': return t('server.snapshot.digest_mismatch', params);
    case 'incomplete': return t('server.snapshot.incomplete', params);
    default: return t('server.snapshot.technical_refusal', { detail: String(raw || '') });
  }
}
