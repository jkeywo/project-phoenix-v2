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
 * outcome. `ServerMessage::GameOver` carried `{ reason }` and dropped the
 * outcome on the way out, so the flag existed, was authored per world, and was
 * simply not published. This module was written to read it anyway, against the
 * day the wire caught up.
 *
 * It has. `GameOver` now carries `{ reason, outcome }` — `outcome` being
 * `balance::Outcome::as_str`, or `null` for an ending that declared no side —
 * and gui/lobby-state.js parks it on `gameOverOutcome` for the caller to pass
 * in. Nothing below changed to accommodate that, which was the point of
 * writing it this way.
 *
 * The fallbacks stay, because `null` is still a real answer. With no declared
 * outcome the only end signal left is `ShipDestroyed`, which is unambiguously
 * a defeat; anything else ends as ENDED — a neutral, honest frame — rather
 * than guessing victory from prose. String-matching the reason was considered
 * and rejected for the reason balance.rs rejected it: the reason is per-world
 * and often a strings.csv key, so no substring reliably tells a win from a
 * loss.
 *
 * ── The post-mission report (issue #1344) ─────────────────────────────────
 *
 * Victory and defeat are one word, and one word is not enough for a mission
 * that saved the stricken hauler and lost the traffic. So a scenario may
 * author a REPORT — a list of rows, each naming a thing the mission was about
 * and how it ended — and `GameOver` now carries it.
 *
 * When rows are present they REPLACE the win/loss frame rather than sitting
 * under it. A fourth outcome bucket, `reported`, is not a fourth kind of
 * ending: it is this module saying "the rows are the result, read them". A
 * catastrophic Falling Skyway ending that still carries a saved-Lyra row is
 * `reported`, not `defeat`, because framing it as DEFEAT throws away the one
 * thing the crew actually achieved. The declared outcome is still on the wire
 * and still the authored truth about the ending — it simply stops being the
 * headline.
 *
 * Rows arrive as `{ id, heading, outcome, state }` and come out the same way,
 * in the order the author wrote them. Everything text-shaped is a
 * `strings.csv` id the caller resolves, because this module composes no
 * English.
 *
 * There is deliberately **no score, no total, no letter grade and no
 * victory/defeat label** anywhere in a reported view. Those exist — the server
 * keeps a signed score per row and a hidden total — and they are for design's
 * headless output. A crew reading "Lyra Ascending: saved (+6)" is reading a
 * coupon, not an account of what happened to her. The wire row has no score
 * field at all, so this is a shape guarantee rather than a filter somebody has
 * to remember to apply.
 *
 * Pure and DOM-free. Two pages render what it returns and each imports this
 * module to do it: client.html (the phone) calls `gameOverView` for the whole
 * frame, and server.html (the Viewscreen) calls `reportRows` for the rows — its
 * rows ride the HUD payload rather than a `GameOver` message, so it composes its
 * own headline from `game_over_message` and takes only the row normalisation
 * from here. Different callers, ONE definition of what a renderable row is and
 * which `state` words style; that is the part that must not be written twice.
 */

/** The four frames the overlay can wear. */
const HEADLINE = {
  victory: 'client.game_over_victory',
  defeat: 'client.game_over_defeat',
  ended: 'client.game_over_ended',
  reported: 'client.game_over_reported',
};

/** The row states a surface may style on; anything else renders unstyled. */
const ROW_STATES = new Set(['saved', 'lost', 'partial', 'neutral']);

/**
 * Normalise the wire's report rows, dropping anything that could not render.
 *
 * A row needs both String Ids to say anything at all; one missing either would
 * be a blank line on the crew's screen, which says less than not showing the
 * row. An unrecognised `state` KEEPS the row — its two ids still say what
 * happened — and blanks the state, so a surface styling on it falls back to
 * neutral presentation rather than inventing a mood from a word it does not
 * know.
 *
 * EXPORTED (and self-registered as `window.gameOverReportRows`) because it is
 * the whole of what the VIEWSCREEN needs. server.html receives the same rows on
 * its HUD payload rather than on a `GameOver` message, so it has no use for the
 * headline/body half of `gameOverView` below — but the validity filter and the
 * state lower-casing here are precisely the parts that must not exist twice.
 * Until issue #1344's review server.html carried its own row loop with neither,
 * and rendered as a pair of blank lines the row a phone had dropped: the two
 * player surfaces disagreed about what the report SAID. One function, both
 * surfaces, and the disagreement has nowhere left to live.
 *
 * @param {unknown} report
 * @returns {{ id: string, headingId: string, outcomeId: string, state: string }[]}
 */
export function reportRows(report) {
  if (!Array.isArray(report)) return [];
  return report
    .filter((row) => row && typeof row === 'object')
    .map((row) => ({
      id: typeof row.id === 'string' ? row.id : '',
      headingId: typeof row.heading === 'string' ? row.heading : '',
      outcomeId: typeof row.outcome === 'string' ? row.outcome : '',
      state:
        typeof row.state === 'string' && ROW_STATES.has(row.state.toLowerCase())
          ? row.state.toLowerCase()
          : '',
    }))
    .filter((row) => row.headingId !== '' && row.outcomeId !== '');
}

/**
 * @param {{ phase?: string, shipDestroyed?: boolean, reason?: string|null,
 *           outcome?: string|null, scenarioTitle?: string|null,
 *           report?: unknown }} s
 *        `outcome` is the authored victory/defeat flag off the wire
 *        (`ServerMessage::GameOver`), or null/undefined for an ending that
 *        declared no side. `report` is that message's row array — empty or
 *        absent for a scenario that authored none. See the module note.
 * @returns {{ visible: boolean, outcome: 'victory'|'defeat'|'ended'|'reported',
 *             headlineId: string, scenarioName: string,
 *             bodyId: string|null, bodyText: string,
 *             rows: { id: string, headingId: string, outcomeId: string, state: string }[] }}
 *   Exactly one of `bodyId` (a string id the caller resolves) and `bodyText`
 *   (already-resolved prose off the wire) is non-empty. `rows` is empty unless
 *   `outcome` is 'reported'.
 */
export function gameOverView(s = {}) {
  const rows = reportRows(s.report);
  const declared = typeof s.outcome === 'string' ? s.outcome.toLowerCase() : null;
  let outcome;
  // The rows outrank every other signal, a hull death included: a run that
  // ended catastrophically still has an account to give of what it saved.
  if (rows.length > 0) outcome = 'reported';
  else if (declared === 'victory' || declared === 'defeat') outcome = declared;
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
    // A reported ending keeps it too: the closing line says what became of the
    // ship, and the rows say what became of everyone else.
    bodyId: (!reason && s.shipDestroyed) ? 'client.ship_destroyed' : null,
    bodyText: reason,
    rows,
  };
}

// Expose for the classic inline scripts that render this: client.html's shell
// takes the whole view model, server.html's __updateHud takes just the rows.
// Neither page can `import` from a classic script, and both call long after the
// module graph has run.
if (typeof window !== 'undefined') {
  window.gameOverView = gameOverView;
  window.gameOverReportRows = reportRows;
}
