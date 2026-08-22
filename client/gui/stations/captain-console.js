/**
 * gui/stations/captain-console.js — one Captain renderer, four hulls
 * (issue #1235, T4.C3 chunk 3 — final chunk of the console-seam programme).
 *
 * The battleship, cruiser, alliance-destroyer and courier Captain seats each
 * shipped their own inline `render(s)` in their `captain.html`. Captain is
 * the most bespoke Station of all: every hull owns a DIFFERENT bundle of
 * systems behind it (battleship/cruiser: captain alone; destroyer: captain +
 * sensors; courier: captain + shields + power + repair + navigation +
 * comms, its Captain seat absorbing every system the compact hull has no
 * dedicated Station for). The only genuinely shared core across all four is
 * camera/viewscreen, red-alert, the objective list, and the station-damage
 * bar — everything else (the destroyer's sensor radar + scan readout +
 * deadline list + Command-intent-free target-name footer, the courier's
 * shields/power/battery/hull/repair columns and Nav/Comms overlays) is a
 * `variant.tail`.
 *
 * A hull supplies a `variant` object (below) and gets back a
 * `renderStation(s, doc = document)` it hands straight to `initConsole`'s
 * `render`. The same function is importable by a vitest suite.
 *
 * @typedef {object} CaptainVariant
 * @property {function(object): object} [captainView]
 *   Given the (already shape-normalised) console payload, return the "view"
 *   the shared core reads camera/red-alert/objectives fields from. A
 *   flat-family hull (battleship, cruiser) omits this — the core defaults to
 *   `s` itself; a system-id-keyed hull (destroyer, courier) returns
 *   `systemView(s, 'captain', 'viewscreen', 'red-alert')`.
 * @property {object} ids                     element ids present in this hull's markup
 * @property {string} [ids.camera]            `ph-camera-select` id
 * @property {string} [ids.redAlert]          `ph-red-alert` id
 * @property {string} [ids.objectives]        `ph-objective-list` id
 * @property {string} [ids.stationDamage]     `ph-station-damage` id
 * @property {string} [ids.autoBadge]         the AUTO badge id, if this hull mounts one
 * @property {function(Array): Array} [filterCameraViews]
 *   Transform the view's `camera_views` before handing them to the camera
 *   select (the courier pattern — its compact bridge exposes only the Fore
 *   hull view and the authored Cinematic view even if the model carries more).
 * @property {object} [footer]                contact-count footer config (the
 *   battleship/cruiser pattern — omit for a hull whose footer reads
 *   something else, or touches nothing, done in `tail` instead)
 * @property {string} footer.id               the footer element id
 * @property {boolean} [footer.colorize]      tint the footer text by contact count
 * @property {function(object, object): boolean} [autoState]
 *   Compute the AUTO badge state from `(s, view)`. Defaults to
 *   `!!view.captain_auto`. The destroyer combines three views' flags — see
 *   its variant.
 * @property {function(object, object, Document, function): void} [tail]
 *   Bespoke per-hull rendering the shared core does not cover, called with
 *   `(s, view, doc, t)` after the common panels are set.
 */

import { t } from '../strings.js';
import { setAutoState } from '../console-ui.js';

/**
 * Build a Captain `renderStation(s, doc)` for one hull from its `variant`.
 *
 * @param {CaptainVariant} variant
 * @returns {function(object, Document=): void} renderStation
 */
export function makeCaptainRender(variant) {
  const ids = variant.ids || {};

  /**
   * @param {object} s   the (shape-normalised) console payload
   * @param {Document} [doc]  the document to render into; defaults to the
   *   ambient `document` in a browser. A vitest suite passes a jsdom document.
   */
  return function renderStation(s, doc) {
    doc = doc || (typeof document !== 'undefined' ? document : null);
    if (!doc || !s) return;

    // The captain view the panels read from — `s` itself for a flat-family
    // hull, a `systemView(...)` slice for a keyed one.
    const view = variant.captainView ? variant.captainView(s) : s;

    // ── Camera / viewscreen ─────────────────────────────────────────────
    if (ids.camera) {
      const el = doc.getElementById(ids.camera);
      if (el) {
        let views = view.camera_views || [];
        if (variant.filterCameraViews) views = variant.filterCameraViews(views);
        el.state = { views, current_view: view.view_direction || '', auto: !!view.viewscreen_auto };
      }
    }

    // ── Red alert ────────────────────────────────────────────────────────
    if (ids.redAlert) {
      const el = doc.getElementById(ids.redAlert);
      if (el) el.state = { active: !!view.red_alert, hold: !!view.weapons_hold, auto: !!view.red_alert_auto };
    }

    // ── Objectives ───────────────────────────────────────────────────────
    if (ids.objectives) {
      const el = doc.getElementById(ids.objectives);
      if (el) el.state = { objectives: view.objectives || [], boosted_objective_id: view.boosted_objective_id ?? null };
    }

    // ── Station-damage bar ───────────────────────────────────────────────
    // Station-wide, read off the top-level payload (never per-system).
    if (ids.stationDamage) {
      const el = doc.getElementById(ids.stationDamage);
      if (el) el.state = s.own_hull || null;
    }

    // ── Contact-count footer (battleship/cruiser pattern) ────────────────
    renderContactsFooter(variant.footer, view, doc);

    // ── AUTO badge ───────────────────────────────────────────────────────
    if (ids.autoBadge) {
      const el = doc.getElementById(ids.autoBadge);
      if (el) setAutoState(null, el, variant.autoState ? !!variant.autoState(s, view) : !!view.captain_auto);
    }

    // ── Bespoke per-hull tail ────────────────────────────────────────────
    if (variant.tail) variant.tail(s, view, doc, t);
  };
}

/**
 * The shared contact-count footer. Battleship and cruiser each carry one
 * (cruiser additionally tints it); the destroyer's footer reads a locked
 * sensor target's name instead (its own `tail`), and the courier's Captain
 * seat carries no footer target at all — both leave `variant.footer` unset,
 * making this a no-op for them.
 *
 * @param {object|undefined} cfg  the variant's `footer` config, or undefined
 * @param {object} view           the captain view
 * @param {Document} doc
 */
function renderContactsFooter(cfg, view, doc) {
  if (!cfg || !cfg.id) return;
  const el = doc.getElementById(cfg.id);
  if (!el) return;
  const n = (view.blips || []).length;
  el.textContent = n > 0
    ? (n === 1 ? t('console.common.contacts.one', { n: 1 }) : t('console.common.contacts.other', { n }))
    : t('console.common.no_target');
  if (cfg.colorize) el.style.color = n > 0 ? 'var(--ink-dim)' : 'var(--ink-faint)';
}
