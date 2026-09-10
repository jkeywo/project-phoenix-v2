/**
 * Reusable before/after/consequence/eligibility presentation for GM inverse
 * work (issue #1441), shared with the inverse-action panel that follows it.
 *
 * The component's job is to be HONEST about three different things a GM keeps
 * conflating during a live event:
 *
 *  - what the simulation state was and became (before/after), which only a
 *    caller that actually captured it can supply — this module never invents it;
 *  - what the action technically did, versus what crews already witnessed. A
 *    restored hull does not un-see an explosion, and no surface built on this
 *    component is allowed to imply otherwise (PRD #1418 story 29);
 *  - whether an inverse exists AT ALL. Two families now have one (issue #1442:
 *    NPC doctrine and faction relations); every other family says plainly, with
 *    the reason, that it does not. A greyed-out Undo button a GM might press in
 *    a crisis would be worse than no button.
 *
 * It renders presentation only. It submits nothing, owns no state and never
 * opens a dialog.
 */
import { wireText } from './strings.js';

/** An inverse this build really performs. */
export const GM_INVERSE_SUPPORTED = 'supported';
/** An inverse this project intends to build, but has not built yet. */
export const GM_INVERSE_PLANNED = 'planned';
/** An inverse that is not coming, with the reason it cannot exist. */
export const GM_INVERSE_OUT_OF_SCOPE = 'out-of-scope';
/** An action family this build does not recognise at all. */
export const GM_INVERSE_UNKNOWN = 'unknown';

const SUPPORTED = (reasonId) => ({
  supported: true,
  status: GM_INVERSE_SUPPORTED,
  reasonId,
});
const PLANNED = (reasonId) => ({
  supported: false,
  status: GM_INVERSE_PLANNED,
  reasonId,
});
const OUT_OF_SCOPE = (reasonId) => ({
  supported: false,
  status: GM_INVERSE_OUT_OF_SCOPE,
  reasonId,
});

/**
 * Wire action kind (`GmActionKind`, kebab-case) → what undo can do about it.
 *
 * A closed table rather than a default, so adding a `GmActionKind` in Rust
 * forces an explicit answer here instead of silently inheriting one. The three
 * `planned` families are the ones PRD #1420 scopes an inverse for; every other
 * entry states why reversing it is not a thing that can be built.
 *
 * A `supported` row is a claim this build can make good on: the canonical
 * reducer has a typed inverse for it AND the journal records the exact
 * before/after pair that inverse needs. Everything else states why not.
 */
export const GM_INVERSE_SUPPORT = Object.freeze({
  'npc-doctrine': SUPPORTED('server.gm.inverse.supported.npc_doctrine'),
  'faction-relation': SUPPORTED('server.gm.inverse.supported.faction_relation'),
  'world-spawn': PLANNED('server.gm.inverse.unavailable.planned'),
  'world-despawn': PLANNED('server.gm.inverse.unavailable.planned'),
  // An inverse is itself an ordinary journal entry, and reversing one would be
  // a redo rather than an undo. Ask for the state you want instead.
  'action-undo': OUT_OF_SCOPE('server.gm.inverse.unavailable.inverse'),
  'session-pause': OUT_OF_SCOPE('server.gm.inverse.unavailable.absolute_state'),
  'station-puppet': OUT_OF_SCOPE('server.gm.inverse.unavailable.absolute_state'),
  'objective-control': OUT_OF_SCOPE('server.gm.inverse.unavailable.absolute_state'),
  'contact-reveal': OUT_OF_SCOPE('server.gm.inverse.unavailable.absolute_state'),
  'contact-conceal': OUT_OF_SCOPE('server.gm.inverse.unavailable.absolute_state'),
  'contact-normal': OUT_OF_SCOPE('server.gm.inverse.unavailable.absolute_state'),
  'system-disable': OUT_OF_SCOPE('server.gm.inverse.unavailable.absolute_state'),
  'system-restore': OUT_OF_SCOPE('server.gm.inverse.unavailable.absolute_state'),
  'station-command': OUT_OF_SCOPE('server.gm.inverse.unavailable.consumed'),
  'event-control': OUT_OF_SCOPE('server.gm.inverse.unavailable.consumed'),
  'direct-effect': OUT_OF_SCOPE('server.gm.inverse.unavailable.folded'),
  comms: OUT_OF_SCOPE('server.gm.inverse.unavailable.witnessed'),
});

const UNKNOWN_KIND = Object.freeze({
  supported: false,
  status: GM_INVERSE_UNKNOWN,
  reasonId: 'server.gm.inverse.unavailable.unknown',
});

/** What undo can do about one action kind. Never throws on an unknown kind. */
export function gmInverseAvailability(actionKind) {
  const entry = (typeof actionKind === 'string' && GM_INVERSE_SUPPORT[actionKind]) || UNKNOWN_KIND;
  return { action_kind: actionKind, ...entry };
}

/**
 * Every action kind this build can actually reverse.
 *
 * Deliberately derived from the table rather than written out, so it cannot
 * claim support the table does not describe. It is empty today, and a caller
 * asking "can I offer Undo anywhere yet?" gets that answer without a special
 * case.
 */
export function gmInverseSupportedKinds() {
  return Object.entries(GM_INVERSE_SUPPORT)
    .filter(([, value]) => value.supported)
    .map(([kind]) => kind);
}

/**
 * Describe one `GmAffectedField` as the pair of sentences the preview shows.
 *
 * The shape is the wire's own externally tagged enum, echoed straight out of
 * the journal projection, so this reads it rather than re-deriving it. An
 * unrecognised variant returns `null`: the caller then renders its own
 * "this caller captured no state" fallback instead of inventing a description
 * of a field this build does not know.
 */
export function gmAffectedFieldText(affected, t = (id) => id, displayText = wireText) {
  if (!affected || typeof affected !== 'object') return null;
  const doctrine = affected['npc-doctrine'];
  if (doctrine && typeof doctrine === 'object' && typeof doctrine.entity === 'string') {
    const value = (id) => (typeof id === 'string' && id.length > 0
      ? t('server.gm.inverse.doctrine_value', { doctrine: displayText(id, id) })
      : t('server.gm.inverse.doctrine_authored'));
    return {
      subject: t('server.gm.inverse.subject_entity', {
        entity: displayText(doctrine.entity, doctrine.entity),
      }),
      before: value(doctrine.before),
      after: value(doctrine.after),
    };
  }
  const relation = affected['faction-hostility'];
  if (relation && typeof relation === 'object'
      && typeof relation.faction === 'string' && typeof relation.enemy === 'string') {
    const value = (hostile) => t(hostile
      ? 'server.gm.inverse.hostility_hostile'
      : 'server.gm.inverse.hostility_neutral');
    return {
      subject: t('server.gm.inverse.subject_relation', {
        faction: displayText(relation.faction, relation.faction),
        enemy: displayText(relation.enemy, relation.enemy),
      }),
      before: value(relation.before === true),
      after: value(relation.after === true),
    };
  }
  return null;
}

export function createGmInversePreview({
  doc = globalThis.document,
  t = (id) => id,
  displayText = wireText,
} = {}) {
  let last = null;

  function row(list, labelId, text, className) {
    const term = doc.createElement('dt');
    term.textContent = t(labelId);
    const value = doc.createElement('dd');
    if (className) value.className = className;
    value.textContent = text;
    list.append(term, value);
    return value;
  }

  /**
   * Paint one descriptor into `container`, replacing whatever was there.
   *
   * `descriptor.before`/`after` are optional captured state; absent means THIS
   * CALLER did not capture it, which is rendered as such rather than as an
   * empty row a reader would take for "nothing changed".
   *
   * The absence says nothing about the action family. Several families do
   * record their own before/after deep in the simulation (`direct-effect`
   * carries its milli-HP delta on `LoggedGmAction::effect`), and three families
   * have an inverse planned; a row that read "no inverse could restore one"
   * here would be false for the first group and would flatly contradict the
   * eligibility row for the second. Whether an inverse can exist is stated once
   * only, by `GM_INVERSE_SUPPORT` in the eligibility row below.
   */
  function render(container, descriptor) {
    const availability = gmInverseAvailability(descriptor?.actionKind);
    if (!container) {
      last = availability;
      return availability;
    }
    container.replaceChildren();
    const list = doc.createElement('dl');
    list.className = 'gm-inverse-rows';
    // A caller may hand over the wire `affected` object verbatim; describing it
    // here keeps the one vocabulary for before/after in one place.
    const described = gmAffectedFieldText(descriptor?.affected, t, displayText);
    const state = (value, fallback) => (typeof value === 'string' && value.length > 0
      ? displayText(value, value)
      : (fallback || t('server.gm.inverse.state_uncaptured')));
    if (described) {
      row(list, 'server.gm.inverse.subject', described.subject);
    }
    row(list, 'server.gm.inverse.before', state(descriptor?.before, described?.before));
    row(list, 'server.gm.inverse.after', state(descriptor?.after, described?.after));
    row(
      list,
      'server.gm.inverse.technical',
      typeof descriptor?.technical === 'string' && descriptor.technical.length > 0
        ? descriptor.technical
        : t('server.gm.inverse.technical_unknown'),
    );
    // Never optional and never caller-suppressible: the one sentence that stops
    // a restore from reading as an erasure of what players know.
    row(
      list,
      'server.gm.inverse.witnessed',
      typeof descriptor?.witnessed === 'string' && descriptor.witnessed.length > 0
        ? descriptor.witnessed
        : t('server.gm.inverse.witnessed_note'),
    );
    const eligibility = row(
      list,
      'server.gm.inverse.eligibility',
      `${t(availability.supported
        ? 'server.gm.inverse.available'
        : 'server.gm.inverse.unavailable')} ${t(availability.reasonId)}`,
      'gm-inverse-eligibility',
    );
    // Status is carried by the sentence above, not by colour alone; the data
    // attributes exist for styling and for tests, never as the only signal.
    eligibility.dataset.supported = String(availability.supported);
    eligibility.dataset.status = availability.status;
    container.append(list);
    last = availability;
    return availability;
  }

  function clear(container) {
    last = null;
    if (container) container.replaceChildren();
  }

  return { render, clear, state: () => last };
}
