/**
 * gui/game-over-view.js — how a session ends (PRD #1023 module 4, user story
 * 17: "I want the game-over screen to frame the outcome with the scenario's
 * name and result, so that the session lands with an ending rather than a
 * dialog box").
 *
 * It ended as a dialog box, and worse than that: the overlay rendered EITHER
 * the hull-death string OR the scenario's authored closing message, never
 * both. Blow up and you were told "Ship Destroyed" while the world's own
 * defeat prose — the one piece of writing that says what the loss meant — was
 * dropped on the floor. Nothing named the scenario, so a crew that had just
 * played Falling Skyway read a sentence that could have come from any world.
 *
 * ── The outcome, and what the wire actually knows ─────────────────────────
 *
 * Scenario authors DO declare the outcome: `game_over(message, "victory")` in
 * a world's script, parsed into `balance::Outcome` and latched on the
 * `GameOverReason` resource, which is a two-field resource — reason AND
 * outcome. `ServerMessage::GameOver` then carries `{ reason }` and drops the
 * outcome on the way out. So the flag exists, is authored per world, and is
 * simply not published.
 *
 * Publishing it is a one-field change to src/core/messages.rs, which is
 * outside a visual/UX pass. This module is therefore written to READ the field
 * if it is ever there (`outcome`, absent today, hence undefined, hence
 * ignored) and to fall back on the only end signal the wire does publish:
 * `ShipDestroyed`, which is unambiguously a defeat. Anything else ends as
 * ENDED — a neutral, honest frame — rather than guessing victory from prose.
 * String-matching the reason was considered and rejected for the same reason
 * balance.rs rejected it: the reason is per-world and often a strings.csv key,
 * so no substring reliably tells a win from a loss.
 *
 * Pure and DOM-free; client.html renders the result.
 */

/** The three frames the overlay can wear. */
const HEADLINE = {
  victory: 'client.game_over_victory',
  defeat: 'client.game_over_defeat',
  ended: 'client.game_over_ended',
};

/**
 * @param {{ phase?: string, shipDestroyed?: boolean, reason?: string|null,
 *           outcome?: string|null, scenarioTitle?: string|null }} s
 *        `outcome` is the authored victory/defeat flag IF the wire ever
 *        carries it (see the module note); today it is always undefined.
 * @returns {{ visible: boolean, outcome: 'victory'|'defeat'|'ended',
 *             headlineId: string, scenarioName: string,
 *             bodyId: string|null, bodyText: string }}
 *   Exactly one of `bodyId` (a string id the caller resolves) and `bodyText`
 *   (already-resolved prose off the wire) is non-empty.
 */
export function gameOverView(s = {}) {
  const declared = typeof s.outcome === 'string' ? s.outcome.toLowerCase() : null;
  let outcome;
  if (declared === 'victory' || declared === 'defeat') outcome = declared;
  else if (s.shipDestroyed) outcome = 'defeat';
  else outcome = 'ended';

  const reason = (s.reason || '').trim();

  return {
    visible: s.phase === 'GameOver',
    outcome,
    headlineId: HEADLINE[outcome],
    scenarioName: s.scenarioTitle || '',
    // The world's own closing prose is the body whenever it authored one —
    // including on a hull death, which is precisely the case the old overlay
    // threw it away for. Only a silent world falls back to the built-in line.
    bodyId: (!reason && s.shipDestroyed) ? 'client.ship_destroyed' : null,
    bodyText: reason,
  };
}

// Expose for the classic inline script in client.html.
if (typeof window !== 'undefined') {
  window.gameOverView = gameOverView;
}
