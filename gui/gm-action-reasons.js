/**
 * The one wire-reason → String Table id table for typed GM actions.
 *
 * Rust's `GmActionRefusalReason` spellings stay machine-readable on the wire
 * while their copy is localised, and every GM surface reads the SAME table:
 * the Session controls (#1292), the mission panel (#1301) and the direct-effect
 * panel (#1310, #1311) must not drift into three vocabularies for one refusal.
 * A reason with no row here falls back to the surface's own `reason_unknown`
 * sentence, which preserves the diagnostic identity rather than hiding it.
 */
export const GM_ACTION_REFUSAL_REASON_LABELS = Object.freeze({
  'not-in-fleet': 'server.gm.session.reason.not_in_fleet',
  'not-game-master': 'server.gm.session.reason.not_game_master',
  'operator-mismatch': 'server.gm.session.reason.operator_mismatch',
  'invalid-operator': 'server.gm.session.reason.invalid_operator',
  'origin-mismatch': 'server.gm.session.reason.origin_mismatch',
  'conflicting-grant': 'server.gm.session.reason.conflicting_grant',
  'non-contiguous-sequence': 'server.gm.session.reason.non_contiguous_sequence',
  'journal-full': 'server.gm.session.reason.journal_full',
  'unreadable-request': 'server.gm.session.reason.unreadable_request',
  'wrong-phase': 'server.gm.session.reason.wrong_phase',
  'unknown-gm-event': 'server.gm.session.reason.unknown_gm_event',
  'protected-entity': 'server.gm.session.reason.protected_entity',
  'unknown-entity': 'server.gm.session.reason.unknown_entity',
  'unknown-objective': 'server.gm.session.reason.unknown_objective',
  'objective-not-active': 'server.gm.session.reason.objective_not_active',
  'objective-scope-mismatch': 'server.gm.session.reason.objective_scope_mismatch',
  'target-not-damageable': 'server.gm.session.reason.target_not_damageable',
  // The two narrowed-scope refusals (issue #1311). They join this table rather
  // than growing a direct-effect-only one: a GM who aims at a Station that is
  // not there must read the same sentence wherever the answer surfaces.
  'unknown-station': 'server.gm.session.reason.unknown_station',
  'unknown-system': 'server.gm.session.reason.unknown_system',
  'unknown-gm-palette-entry': 'server.gm.session.reason.unknown_gm_palette_entry',
  'world-unavailable': 'server.gm.session.reason.world_unavailable',
});

/** The immediate browser-to-WASM ingress refusal, which has no wire reason. */
export const LOCAL_INGRESS_REFUSAL = 'ingress-rejected';
