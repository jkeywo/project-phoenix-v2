/**
 * gui/scenario-pick.js — what a phone's scenario/ship tap looks like while it
 * is still in the air (PRD #1023 module 4, user story 16).
 *
 * The selection rule itself is not here. It lives on the host, in
 * gui/scenario-arbiter.js, and it is "first valid request wins" — no voting,
 * no captain authority, and crucially no acknowledgement addressed to the
 * phone that asked. A phone learns the answer the same way every other phone
 * does: the next `ScenarioCatalog` broadcast arrives carrying the lock.
 *
 * That is a perfectly good protocol and a bad experience, because between the
 * tap and the broadcast the picker looked exactly as it did before the tap.
 * On a slow link the player taps again. When someone else's request landed
 * first, the stage changes underneath them with no explanation, and the PRD is
 * blunt about how that reads: "the stage change never feels arbitrary".
 *
 * So the phone keeps one extra piece of local state — the request it sent,
 * unacknowledged — and settles it against each inbound lock:
 *
 *   sent, nothing locked yet   → PENDING. Say so, and stop taking taps.
 *   sent, locked to my choice  → WON.  Move on; the pending state was the
 *                                      whole feedback and it is done.
 *   sent, locked to another    → LOST. Move on, but SAY the other phone won
 *                                      and name what it chose.
 *
 * Pure and DOM-free: client.html owns the rendering, this owns the rules.
 */

/**
 * The local record of an unacknowledged request.
 *
 * @param {'scenario'|'ship'} kind
 * @param {string} id  the scenario id, or the ship's template_path
 * @returns {{ kind: 'scenario'|'ship', id: string }}
 */
export function pendingPick(kind, id) {
  return { kind, id };
}

/** The locked value this pending request is waiting on, or null. */
function lockedValueFor(kind, locked) {
  const l = locked || {};
  if (kind === 'ship') return l.template_path != null ? l.template_path : null;
  return l.scenario_id != null ? l.scenario_id : null;
}

/**
 * Settle an outstanding request against the authoritative lock.
 *
 * @param {{kind: string, id: string}|null} pending
 * @param {{scenario_id: string|null, template_path: string|null}|null} locked
 * @returns {{ state: 'idle'|'pending'|'won'|'lost',
 *             pending: object|null,
 *             lostTo: string|null }}
 *   `pending` is what the caller should hold next — the same request while it
 *   is still in flight, null once it has been answered either way. `lostTo` is
 *   the id the host actually locked, so the caller can name the winning choice
 *   rather than saying "someone else picked something".
 */
export function settlePick(pending, locked) {
  if (!pending) return { state: 'idle', pending: null, lostTo: null };
  const value = lockedValueFor(pending.kind, locked);
  if (value == null) return { state: 'pending', pending, lostTo: null };
  if (value === pending.id) return { state: 'won', pending: null, lostTo: null };
  return { state: 'lost', pending: null, lostTo: value };
}

/**
 * Which of the picker's three stages the phone is in.
 *
 * Identical to the rule client.html applied inline; named here so the pending
 * logic and the stage logic can be tested as one thing.
 *
 * @returns {'scenario'|'ship'|'locked'}
 */
export function pickStage(locked) {
  const l = locked || {};
  if (l.scenario_id == null) return 'scenario';
  if (l.template_path == null) return 'ship';
  return 'locked';
}

/**
 * The full picker view model.
 *
 * @param {{ catalog?: Array, locked?: object, pending?: object|null,
 *           notice?: {choice: string}|null }} input
 *        `notice` is the settled-LOST record the caller kept from the previous
 *        broadcast; it survives one render so the message lands on the stage
 *        the player was moved to, not the one they left.
 * @returns {object} view model — see the return literal.
 */
export function scenarioPickView(input = {}) {
  const catalog = input.catalog || [];
  const locked = input.locked || { scenario_id: null, template_path: null };
  const pending = input.pending || null;
  const notice = input.notice || null;
  const stage = pickStage(locked);

  // A tap is only in flight for the stage that issued it. A scenario request
  // that has already been answered cannot grey out the ship buttons.
  const busy = !!pending && (
    (pending.kind === 'scenario' && stage === 'scenario')
    || (pending.kind === 'ship' && stage === 'ship')
  );

  return {
    stage,
    // Whether taps are accepted. One request at a time: the arbiter ignores
    // the second anyway, and a picker that keeps accepting taps it will not
    // act on is the thing that made the stage change feel arbitrary.
    accepting: !busy,
    busy,
    // The id of the specific control that should read as pending.
    pendingId: busy ? pending.id : null,
    // The empty-catalogue case. It is NOT "waiting for the host to select a
    // scenario" — the host has broadcast its catalogue and the catalogue has
    // nothing in it. Saying the former told the player to wait for something
    // that had already happened.
    emptyId: (stage === 'scenario' && catalog.length === 0) ? 'client.no_scenarios' : null,
    // Another phone won the race. Carries the winning choice so the caller can
    // name it.
    noticeId: notice ? 'client.pick_taken' : null,
    noticeParams: notice ? { choice: notice.choice } : null,
  };
}

// Expose for the classic inline script in client.html.
if (typeof window !== 'undefined') {
  window.scenarioPick = { pendingPick, settlePick, pickStage, scenarioPickView };
}
