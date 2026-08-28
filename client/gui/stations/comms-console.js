/**
 * gui/stations/comms-console.js — one Comms renderer, two hulls (issue
 * #1235, T4.C3 chunk 3 — final chunk of the console-seam programme).
 *
 * Only the battleship and cruiser mount a dedicated Comms Station (the
 * destroyer and courier have none — see the per-hull ship TOMLs). The two shipped their
 * own inline `render(s)` in `comms.html`. The shared core is the contact
 * list, hail list, current-message thread and station-damage bar — every
 * hull mounts those four with the same ids. The cruiser's Comms seat also
 * absorbs Navigation (its own Station elsewhere on the battleship), so its
 * footer reads a waypoint and it carries a Nav overlay + message-count
 * footer-right the battleship has neither of; that divergence lives entirely
 * in its `tail`, alongside the battleship's simpler hail-name footer.
 *
 * A hull supplies a `variant` object (below) and gets back a
 * `renderStation(s, doc = document)` it hands straight to `initConsole`'s
 * `render`. The same function is importable by a vitest suite.
 *
 * @typedef {object} CommsVariant
 * @property {function(object): object} [commsView]
 *   Given the (already shape-normalised) console payload, return the "view"
 *   the shared core reads contacts/messages fields from. The battleship
 *   omits this (flat `comms` family payload — the core defaults to `s`
 *   itself); the cruiser returns `familyView(s, 'comms')`.
 * @property {object} ids                     element ids present in this hull's markup
 * @property {string} [ids.contactList]       `ph-comms-contact-list` id
 * @property {string} [ids.hailList]          `ph-comms-hail-list` id
 * @property {string} [ids.currentMessage]    `ph-comms-current-message` id
 * @property {string} [ids.stationDamage]     `ph-station-damage` id
 * @property {string} [ids.autoBadge]         the AUTO badge id
 * @property {function(object, object): boolean} [autoState]
 *   Compute the AUTO badge state from `(s, view)`. Defaults to
 *   `!!view.comms_auto`. The cruiser conjuncts Navigation's own auto flag —
 *   see its variant.
 * @property {function(object, object, Document, function): void} [tail]
 *   Bespoke per-hull rendering the shared core does not cover — the
 *   footer-target text (hail name vs. waypoint name — the two hulls read
 *   different sources for it, so there is no shared footer helper here),
 *   called with `(s, view, doc, t)` after the common panels are set.
 */

import { t } from '../strings.js';
import { setAutoState } from '../console-ui.js';
import { isLatestLiveCriticalMessage } from '../comms-state.js';

/**
 * Build a Comms `renderStation(s, doc)` for one hull from its `variant`.
 *
 * @param {CommsVariant} variant
 * @returns {function(object, Document=): void} renderStation
 */
export function makeCommsRender(variant) {
  const ids = variant.ids || {};

  /**
   * @param {object} s   the (shape-normalised) console payload
   * @param {Document} [doc]  the document to render into; defaults to the
   *   ambient `document` in a browser. A vitest suite passes a jsdom document.
   */
  return function renderStation(s, doc) {
    doc = doc || (typeof document !== 'undefined' ? document : null);
    if (!doc || !s) return;

    // The comms view the panels read from — `s` itself for the battleship's
    // flat `comms` family, a metadata-selected family slice for the cruiser.
    const view = variant.commsView ? variant.commsView(s) : s;

    // ── Contact list ─────────────────────────────────────────────────────
    if (ids.contactList) {
      const el = doc.getElementById(ids.contactList);
      if (el) el.state = { contacts: view.contacts || [] };
    }

    // ── Hail list ────────────────────────────────────────────────────────
    if (ids.hailList) {
      const el = doc.getElementById(ids.hailList);
      if (el) el.state = view;
    }

    // ── Current-message thread ──────────────────────────────────────────
    const msgs = view.messages || [];
    // Critical remains ordinary, non-modal panel content, but it wins the
    // panel's existing automatic selection while it is live.
    const threadMsg = [...msgs].reverse().find(m => isLatestLiveCriticalMessage(m, msgs))
      || msgs.find((m) => !m.is_read)
      || msgs[msgs.length - 1]
      || null;
    if (ids.currentMessage) {
      const el = doc.getElementById(ids.currentMessage);
      if (el) el.state = { thread: threadMsg, messages: msgs, rejection: view.rejection };
    }

    // ── Station-damage bar ───────────────────────────────────────────────
    // Station-wide, read off the top-level payload (never per-system).
    if (ids.stationDamage) {
      const el = doc.getElementById(ids.stationDamage);
      if (el) el.state = s.own_hull || null;
    }

    // ── AUTO badge ───────────────────────────────────────────────────────
    if (ids.autoBadge) {
      const el = doc.getElementById(ids.autoBadge);
      if (el) setAutoState(null, el, variant.autoState ? !!variant.autoState(s, view) : !!view.comms_auto);
    }

    // ── Bespoke per-hull tail (footer target text, Nav overlay, ...) ─────
    if (variant.tail) variant.tail(s, view, doc, t);
  };
}
