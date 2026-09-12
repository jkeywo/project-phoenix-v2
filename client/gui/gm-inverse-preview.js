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
 *  - whether an inverse exists AT ALL. Four families now have one (#1442: NPC
 *    doctrine and faction relations; #1443: a placement; #1444: an allowed
 *    removal); every other family says plainly, with the reason, that it does
 *    not. A greyed-out Undo button a GM might press in a crisis would be worse
 *    than no button.
 *
 * A placement adds a fourth honest thing to say: how long crews have had it in
 * sensor range, and that at two cumulative simulation seconds the chance to
 * take it back is gone for good (issue #1443, PRD #1420 story 4). That is a
 * fact about ONE entry rather than about its family, so it arrives per row and
 * narrows the family answer rather than replacing it. A removal narrows its own
 * family answer the same way, through the journal row's `capture_lost`.
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
/**
 * An inverse this build performs, whose window on THIS entry has closed.
 *
 * Distinct from `out-of-scope`, and the distinction matters to a GM: the family
 * is reversible and the next placement will be too. This one was seen.
 */
export const GM_INVERSE_EXPIRED = 'expired';

const SUPPORTED = (reasonId, exposure = false) => ({
  supported: true,
  status: GM_INVERSE_SUPPORTED,
  reasonId,
  // Whether an entry of this family can only be offered once its live exposure
  // is known. A placement can: the answer changes second by second, and a
  // control offered without it would be a guess.
  exposure,
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
  'world-spawn': SUPPORTED('server.gm.inverse.supported.world_spawn', true),
  // A removal's inverse is not a value to write back but a whole entity to
  // rebuild, so it is supported only while the run still holds the capture the
  // rebuild reads. `capture_lost` on the journal row is that half of the answer
  // and the panel consults it; this table answers the family-level question,
  // which is that this build really does reverse removals.
  'world-despawn': SUPPORTED('server.gm.inverse.supported.world_despawn'),
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
  // A restore (#1446) replaced the entire world, including the journal this
  // entry would have to be read back out of. There is no value to write back
  // and nothing that could be called an inverse; the way back is another
  // restore, which is a fresh decision rather than a reversal of this one.
  'live-restore': OUT_OF_SCOPE('server.gm.inverse.unavailable.live_restore'),
  comms: OUT_OF_SCOPE('server.gm.inverse.unavailable.witnessed'),
});

const UNKNOWN_KIND = Object.freeze({
  supported: false,
  status: GM_INVERSE_UNKNOWN,
  reasonId: 'server.gm.inverse.unavailable.unknown',
});

/**
 * The live sensor-exposure status of one placement, as `GmSpawnExposureStatus`
 * puts it on the wire.
 *
 * Strict: a partial or malformed object is `null`, never a half-believed one.
 * Absence is a real answer here — this peer may keep no stopwatch — and the
 * caller must treat it as "not known" rather than as "not exposed".
 */
export function normaliseGmSpawnExposure(value) {
  if (!value || typeof value !== 'object') return null;
  const ms = (n) => Number.isSafeInteger(n) && n >= 0;
  if (!ms(value.exposed_ms) || !ms(value.limit_ms) || typeof value.latched !== 'boolean') {
    return null;
  }
  return { exposed_ms: value.exposed_ms, limit_ms: value.limit_ms, latched: value.latched };
}

/**
 * What undo can do about one action kind, narrowed by this entry's own live
 * exposure. Never throws on an unknown kind or a malformed status.
 *
 * The family table decides whether an inverse can exist; `exposure` decides
 * whether THIS one may still be asked for. Neither is authority: the canonical
 * reducer answers again at the apply tick, and this only decides whether it is
 * honest to offer a control.
 */
export function gmInverseAvailability(actionKind, exposure) {
  const entry = (typeof actionKind === 'string' && GM_INVERSE_SUPPORT[actionKind]) || UNKNOWN_KIND;
  if (!entry.supported || !entry.exposure) return { action_kind: actionKind, ...entry };
  const seen = normaliseGmSpawnExposure(exposure);
  if (!seen) {
    return {
      action_kind: actionKind,
      supported: false,
      status: GM_INVERSE_UNKNOWN,
      reasonId: 'server.gm.inverse.unavailable.exposure_unknown',
      exposure: true,
    };
  }
  if (seen.latched) {
    return {
      action_kind: actionKind,
      supported: false,
      status: GM_INVERSE_EXPIRED,
      reasonId: 'server.gm.inverse.unavailable.sensor_exposure',
      exposure: true,
    };
  }
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
  // Presence, from either end: a placement that was added (issue #1443) and a
  // removal that took one away (issue #1444) are the same pair read in opposite
  // directions, and they share the two value sentences for that reason.
  const inWorld = (present) => t(present
    ? 'server.gm.inverse.presence_present'
    : 'server.gm.inverse.presence_absent');
  const placement = affected['spawned-entity'];
  if (placement && typeof placement === 'object' && typeof placement.name === 'string') {
    return {
      subject: t('server.gm.inverse.subject_placement', {
        name: displayText(placement.name, placement.name),
      }),
      before: inWorld(placement.before === true),
      after: inWorld(placement.after === true),
    };
  }
  // A removal's pair is the ENTITY'S PRESENCE and nothing more: the
  // simulation-side capture holds the state, because a whole hull is not
  // something a browser echoes back on every Undo press.
  const presence = affected['entity-presence'];
  if (presence && typeof presence === 'object' && typeof presence.entity === 'string') {
    return {
      subject: t('server.gm.inverse.subject_presence', {
        entity: displayText(presence.entity, presence.entity),
      }),
      before: inWorld(presence.before === true),
      after: inWorld(presence.after === true),
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

/** Whether one wire `affected` object describes an entity's presence. */
export function affectedPresence(affected) {
  const presence = affected && typeof affected === 'object' && affected['entity-presence'];
  return !!presence && typeof presence === 'object' && typeof presence.entity === 'string';
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
    const availability = gmInverseAvailability(descriptor?.actionKind, descriptor?.exposure);
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
    // a restore from reading as an erasure of what players know. A removal adds
    // a second sentence naming the KINDS of thing its cleanup released — locks,
    // tows, docking, transports, console selections — which come back only when
    // a crew re-makes them, because "crews remember" is not specific enough to
    // plan around. The same words for every removal, keyed on the affected pair
    // alone: which of them this particular hull held is simulation-side detail
    // the capture deliberately does not carry (see `src/gm_despawn_undo.rs`).
    row(
      list,
      'server.gm.inverse.witnessed',
      typeof descriptor?.witnessed === 'string' && descriptor.witnessed.length > 0
        ? descriptor.witnessed
        : `${t('server.gm.inverse.witnessed_note')}${affectedPresence(descriptor?.affected)
          ? ` ${t('server.gm.inverse.witnessed_removal')}`
          : ''}`,
    );
    // How much of the two seconds is gone, in words and in numbers, whether or
    // not the window is still open. A GM deciding whether to take a placement
    // back needs the count BEFORE it runs out, not the refusal after.
    // Gated on the FAMILY having a window, not merely on a payload arriving: a
    // doctrine change has no two-second clock, and drawing one over it would
    // invent a deadline the reducer does not enforce.
    const seen = availability.exposure ? normaliseGmSpawnExposure(descriptor?.exposure) : null;
    if (seen) {
      const secs = (ms) => (ms / 1000).toFixed(1);
      const exposure = row(
        list,
        'server.gm.inverse.exposure',
        seen.latched
          ? t('server.gm.inverse.exposure_elapsed', { limit: secs(seen.limit_ms) })
          : t('server.gm.inverse.exposure_remaining', {
            elapsed: secs(seen.exposed_ms),
            limit: secs(seen.limit_ms),
          }),
        'gm-inverse-exposure',
      );
      exposure.dataset.latched = String(seen.latched);
    }
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
