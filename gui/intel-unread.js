/**
 * gui/intel-unread.js — how much Intel arrived since the seat last looked at it
 * (issue #1373, PRD #1371).
 *
 * The Intel panel is an overlay: the seat only sees it when it deliberately
 * opens the tab, so a dossier can gain a fact while nobody is reading. The bar
 * carries the count of SUBJECTS that grew, not of individual facts, because
 * that is the number the tab can act on — "three files have something new"
 * sends you to the list, "eleven new lines" does not tell you where.
 *
 * Two pure functions and a baseline the caller holds. There is no class and no
 * module-level state here on purpose: the "seen" baseline belongs to the
 * document that is showing the panel, not to this module, and a console that
 * reloads starts from an empty baseline exactly as a player who has never
 * opened the panel does. That is the honest answer — a seat joining mid-mission
 * genuinely has not read the three files already on record.
 *
 * The per-subject arithmetic mirrors `#countLabel` in
 * `gui/components/ph-dossier-panel.js`: facts plus evidence, which is what the
 * list row itself shows. The two must agree — a badge that counts something the
 * panel does not display is a badge that never clears.
 */

/**
 * How much is on file for one subject: facts plus evidence, the same sum the
 * dossier list row prints.
 *
 * @param {{facts?: Array, evidence?: Array}|null|undefined} dossier
 * @returns {number}
 */
export function dossierEntryCount(dossier) {
  if (!dossier || typeof dossier !== 'object') return 0;
  const facts = Array.isArray(dossier.facts) ? dossier.facts.length : 0;
  const evidence = Array.isArray(dossier.evidence) ? dossier.evidence.length : 0;
  return facts + evidence;
}

/**
 * How many subjects have grown since the baseline was taken.
 *
 * A subject the baseline has never heard of counts as unread as soon as it has
 * anything on file; an empty new subject does not, because there is nothing to
 * read yet (the panel itself says "nothing on file" for exactly that case).
 * A subject that SHRANK is not unread — the count only ever answers "is there
 * something here I have not seen", never "did something change".
 *
 * @param {Array<{uuid?: string, facts?: Array, evidence?: Array}>|null|undefined} subjects
 * @param {Object<string, number>|null|undefined} seen  baseline from markIntelSeen
 * @returns {number}
 */
export function intelUnreadCount(subjects, seen) {
  const baseline = seen || {};
  let unread = 0;
  for (const subject of Array.isArray(subjects) ? subjects : []) {
    const key = subject && typeof subject.uuid === 'string' ? subject.uuid : '';
    if (key === '') continue;
    const known = Number.isFinite(baseline[key]) ? baseline[key] : 0;
    if (dossierEntryCount(subject) > known) unread += 1;
  }
  return unread;
}

/**
 * The baseline after the seat has looked: every current subject recorded at the
 * size it is now.
 *
 * Subjects the payload no longer carries keep their old entry rather than being
 * dropped, so a subject that leaves the recipient's projection and comes back
 * unchanged does not re-announce itself. Returns a NEW object; the argument is
 * never mutated.
 *
 * @param {Array<{uuid?: string, facts?: Array, evidence?: Array}>|null|undefined} subjects
 * @param {Object<string, number>|null|undefined} seen  previous baseline, if any
 * @returns {Object<string, number>}
 */
export function markIntelSeen(subjects, seen) {
  const next = Object.assign({}, seen || {});
  for (const subject of Array.isArray(subjects) ? subjects : []) {
    const key = subject && typeof subject.uuid === 'string' ? subject.uuid : '';
    if (key === '') continue;
    next[key] = dossierEntryCount(subject);
  }
  return next;
}
