/**
 * Readable presentation of the shared live-restore candidate preflight
 * (issue #1445).
 *
 * The DECISION is not here. `src/gm_checkpoint.rs` decides whether one
 * peer-local catalogue row could hold this session's current ship/Station
 * assignments, and issue #1446's restore revalidates authoritatively against
 * that same model. This module only normalises the answer that crosses
 * wasm-bindgen and turns each block into a sentence — so the picker and the
 * restore flow cannot end up describing compatibility differently.
 *
 * A row with no `preflight` at all is the pre-boot landing catalogue: there is
 * no live session to be a candidate for, so `null` is returned rather than a
 * fabricated verdict.
 */

/** Block kinds this build knows how to spell out. Mirrors `CandidateBlock`. */
const BLOCK_LABELS = Object.freeze({
  unreadable: 'server.gm.checkpoint.block.unreadable',
  'no-fleet-record': 'server.gm.checkpoint.block.no_fleet_record',
  'scenario-differs': 'server.gm.checkpoint.block.scenario_differs',
  'format-moved': 'server.gm.checkpoint.block.format_moved',
  'rules-moved': 'server.gm.checkpoint.block.rules_moved',
  'content-moved': 'server.gm.checkpoint.block.content_moved',
  'content-unverified': 'server.gm.checkpoint.block.content_unverified',
  'missing-ship': 'server.gm.checkpoint.block.missing_ship',
  'hull-differs': 'server.gm.checkpoint.block.hull_differs',
  'hull-unknown': 'server.gm.checkpoint.block.hull_unknown',
});

const text = (value) => (typeof value === 'string' ? value : '');
const strings = (value) => (Array.isArray(value) ? value.filter((id) => text(id) !== '') : []);

/**
 * Normalise one `preflight` object from a catalogue row.
 *
 * Returns `null` when the row carries none. An object whose `eligible` is not
 * a boolean is treated as a refusal with no stated reason rather than as an
 * eligible candidate: an unparsable verdict must never read as a green light.
 */
export function normalizeCandidatePreflight(value) {
  if (!value || typeof value !== 'object') return null;
  const blocks = Array.isArray(value.blocks) ? value.blocks : [];
  const normalised = [];
  for (const block of blocks) {
    if (!block || typeof block !== 'object' || !text(block.kind)) continue;
    normalised.push({
      kind: block.kind,
      ...(block.candidate === undefined || block.candidate === null
        ? {} : { candidate: text(block.candidate) }),
      ...(block.live === undefined || block.live === null
        ? {} : { live: text(block.live) }),
      ...(Number.isSafeInteger(block.slot) ? { slot: block.slot } : {}),
      ...(strings(block.stations).length ? { stations: strings(block.stations) } : {}),
    });
  }
  // `eligible` is published as its own field precisely so nothing has to infer
  // it, but a verdict that disagrees with its own reasons is not trustworthy:
  // the reasons win, because they are what a GM can actually read and act on.
  const eligible = value.eligible === true && normalised.length === 0;
  return { eligible, blocks: normalised };
}

/**
 * One readable sentence for one block.
 *
 * A kind this build does not know keeps its wire identity instead of vanishing:
 * `CandidateBlock` is append-only, and a picker that silently dropped a newer
 * host's refusal would show a save as eligible for a reason it cannot name.
 */
export function candidateBlockText(block, t) {
  const hull = (value) => (text(value) === ''
    ? t('server.gm.checkpoint.hull_unspecified')
    : value);
  switch (block.kind) {
    case 'scenario-differs':
      return t(BLOCK_LABELS[block.kind], {
        candidate: text(block.candidate),
        live: text(block.live),
      });
    case 'missing-ship':
      return t(block.stations
        ? BLOCK_LABELS[block.kind]
        : 'server.gm.checkpoint.block.missing_ship_uncrewed', {
        slot: String(block.slot ?? ''),
        stations: (block.stations || []).join(', '),
      });
    case 'hull-unknown':
      return t(block.stations
        ? BLOCK_LABELS[block.kind]
        : 'server.gm.checkpoint.block.hull_unknown_uncrewed', {
        slot: String(block.slot ?? ''),
        stations: (block.stations || []).join(', '),
      });
    case 'hull-differs':
      return t(block.stations
        ? BLOCK_LABELS[block.kind]
        : 'server.gm.checkpoint.block.hull_differs_uncrewed', {
        slot: String(block.slot ?? ''),
        candidate: hull(block.candidate),
        live: hull(block.live),
        stations: (block.stations || []).join(', '),
      });
    default:
      return BLOCK_LABELS[block.kind]
        ? t(BLOCK_LABELS[block.kind])
        : t('server.gm.checkpoint.block.unknown', { kind: block.kind });
  }
}

/** Every reason, in the order Rust reported them. */
export function candidateBlockTexts(preflight, t) {
  if (!preflight) return [];
  return preflight.blocks.map((block) => candidateBlockText(block, t));
}

/**
 * Render a preflight verdict into a container, replacing whatever was there.
 *
 * Words, never colour alone (#1418 story 6): the verdict is a sentence, each
 * reason is a list item, and `data-eligible` is the second, redundant signal.
 * Returns the container so a caller can append it in one expression.
 */
export function paintCandidatePreflight(host, preflight, { doc, t }) {
  if (!host) return host;
  host.replaceChildren();
  // Unchecked is its own answer, and it gets a sentence too: a blank panel
  // under a selected row reads as "nothing wrong with it", which is exactly
  // what an unanswered preflight has NOT established.
  host.dataset.eligible = preflight ? String(preflight.eligible) : 'unknown';
  const verdict = doc.createElement('p');
  verdict.className = 'gm-checkpoint-verdict';
  verdict.textContent = t(!preflight
    ? 'server.gm.checkpoint.verdict_unknown'
    : preflight.eligible
      ? 'server.gm.checkpoint.eligible'
      : 'server.gm.checkpoint.ineligible');
  host.appendChild(verdict);
  if (!preflight || preflight.blocks.length === 0) return host;
  const reasons = doc.createElement('ul');
  reasons.className = 'gm-checkpoint-reasons';
  for (const line of candidateBlockTexts(preflight, t)) {
    const item = doc.createElement('li');
    item.textContent = line;
    reasons.appendChild(item);
  }
  host.appendChild(reasons);
  return host;
}
