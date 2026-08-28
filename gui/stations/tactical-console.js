/**
 * gui/stations/tactical-console.js — one Tactical renderer, four hulls
 * (issue #1234, T4.C2 of the console-seam programme).
 *
 * The battleship, cruiser, alliance-destroyer and courier Tactical seats each
 * shipped their own inline `render(s)` in their `tactical.html`. The four were
 * ~80% the same — a radar, phaser/blaster/torpedo controls, a station-damage
 * bar, a target footer and an AUTO badge — differing only in which system
 * families feed the panels, a few ids, and a bespoke tail (the destroyer's
 * dossier + Command-intent advice, the courier's sensors + helm column).
 *
 * Four copies meant four chances to drift, and they had: only the battleship
 * set BOTH `target_uuid` (the inner `ph-radar`'s locked contact) and
 * `selected_target_uuid` (the outer highlight ring) on `ph-tactical-radar`
 * (see gui/components/ph-tactical-radar.js — the two fields drive two different
 * layers). The other three set only `selected_target_uuid`, so their inner
 * radar never showed the locked target. Folding the four onto ONE renderer
 * fixes that divergence by construction: `renderStation` always sets both.
 *
 * A hull supplies a `variant` object (below) and gets back a
 * `renderStation(s, doc = document)` it hands straight to `initConsole`'s
 * `render`. The same function is importable by a vitest suite so the radar
 * contract is asserted, per hull, without a browser.
 *
 * @typedef {object} TacticalVariant
 * @property {function(object): object} weaponsView
 *   Given the (already shape-normalised) console payload, return the weapons
 *   "view" the radar/phaser/blaster/torpedo panels read from. A flat-family
 *   hull (battleship, cruiser) returns `s` itself; a system-id-keyed hull
 *   (destroyer, courier) returns `familyView(s, 'tactical')`.
 * @property {object} ids                     element ids present in this hull's markup
 * @property {string} ids.radar               the `ph-tactical-radar` id (always present)
 * @property {string} [ids.phasers]           `ph-phasers-controls` id, if the hull mounts phasers
 * @property {string} [ids.blasters]          `ph-blasters-controls` id, if the hull mounts blasters
 * @property {string} [ids.torpedo]           `ph-torpedo-controls` id, if the hull mounts tubes
 * @property {string} [ids.stationDamage]     `ph-station-damage` id
 * @property {string} [ids.autoBadge]         the AUTO badge id
 * @property {boolean} [blastersHideWhenEmpty] hide the blaster panel when there are no banks
 *   (the battleship pattern — a hull with no blaster mount shows nothing;
 *   see issue #925). Off by default: a hull that authored a blaster column
 *   keeps it in view even while empty.
 * @property {number} [torpedoMaxDefault]     magazine `max` fallback when the wire
 *   sends neither `torpedo_max` nor `torpedo_count` (20 for the battleship/cruiser
 *   heavies, 0 elsewhere).
 * @property {object} [footer]                target-footer config
 * @property {string} footer.id               the footer target element id
 * @property {boolean} [footer.colorize]      tint fire-bright while a target is locked
 * @property {boolean} [footer.uuidFallbackName] show the raw uuid when a locked target
 *   has no name (the destroyer pattern) rather than the localized "LOCKED" word
 * @property {string} [footer.prefix]         glyph prefixed to a named/uuid target
 * @property {function(object, object, Document, function): void} [tail]
 *   Bespoke per-hull rendering the shared core does not cover, called with
 *   `(s, w, doc, t)` after the common panels are set.
 */

import { t } from '../strings.js';
import { setAutoState } from '../console-ui.js';

/**
 * Build a Tactical `renderStation(s, doc)` for one hull from its `variant`.
 *
 * @param {TacticalVariant} variant
 * @returns {function(object, Document=): void} renderStation
 */
export function makeTacticalRender(variant) {
  const ids = variant.ids || {};

  /**
   * @param {object} s   the (shape-normalised) console payload
   * @param {Document} [doc]  the document to render into; defaults to the
   *   ambient `document` in a browser. A vitest suite passes a jsdom document.
   */
  return function renderStation(s, doc) {
    doc = doc || (typeof document !== 'undefined' ? document : null);
    if (!doc || !s) return;

    // The weapons view the panels read from — `s` itself for a flat-family
    // hull, a metadata-selected family slice for a keyed one.
    const w = variant.weaponsView ? variant.weaponsView(s) : s;

    // ── Radar ────────────────────────────────────────────────────────────
    // BOTH uuids (issue #1234): `target_uuid` reaches the inner `ph-radar` so
    // it draws the locked contact; `selected_target_uuid` drives the outer
    // highlight ring. Setting only the latter — the pre-#1234 bug on three
    // hulls — left the inner radar blind to the lock.
    const radarEl = doc.getElementById(ids.radar);
    if (radarEl) {
      radarEl.state = {
        blips: w.blips || [],
        x: w.ship_x || 0,
        z: w.ship_z || 0,
        speed: w.ship_speed || 0,
        ship_heading: w.ship_heading || 0,
        phaser_arcs: w.phaser_arcs || [],
        torpedo_arcs: w.torpedo_arcs || [],
        selected_target_uuid: w.target_uuid || null,
        target_uuid: w.target_uuid || null,
      };
    }

    // ── Phasers ──────────────────────────────────────────────────────────
    if (ids.phasers) {
      const el = doc.getElementById(ids.phasers);
      if (el) {
        el.state = {
          banks: w.banks || [],
          target_valid: !!w.target_uuid,
          mode: w.phaser_mode || 'Auto',
        };
      }
    }

    // ── Blasters (issue #925) ────────────────────────────────────────────
    // A hull whose Tactical seat owns no blaster bank gets an empty list; the
    // battleship hides the panel when empty, a hull that authored a blaster
    // column keeps it in view.
    if (ids.blasters) {
      const el = doc.getElementById(ids.blasters);
      if (el) {
        const banks = w.blasters || [];
        if (variant.blastersHideWhenEmpty) el.hidden = banks.length === 0;
        el.state = { banks };
      }
    }

    // ── Torpedo ──────────────────────────────────────────────────────────
    if (ids.torpedo) {
      const el = doc.getElementById(ids.torpedo);
      if (el) {
        const dflt = variant.torpedoMaxDefault || 0;
        el.state = {
          tubes: w.tubes || [],
          magazine: { current: w.torpedo_count || 0, max: w.torpedo_max || w.torpedo_count || dflt },
          target_uuid: w.target_uuid || null,
        };
      }
    }

    // ── Station-damage bar ───────────────────────────────────────────────
    // Station-wide, read off the top-level payload (never per-system).
    if (ids.stationDamage) {
      const el = doc.getElementById(ids.stationDamage);
      if (el) el.state = s.own_hull || null;
    }

    // ── Target footer ────────────────────────────────────────────────────
    renderFooter(variant.footer, w, doc);

    // ── AUTO badge ───────────────────────────────────────────────────────
    if (ids.autoBadge) {
      const el = doc.getElementById(ids.autoBadge);
      if (el) setAutoState(null, el, !!w.tactical_auto);
    }

    // ── Bespoke per-hull tail ────────────────────────────────────────────
    if (variant.tail) variant.tail(s, w, doc, t);
  };
}

/**
 * The shared target footer. Three hulls carry one; the courier carries none.
 *
 * @param {object|undefined} cfg  the variant's `footer` config, or undefined
 * @param {object} w             the weapons view
 * @param {Document} doc
 */
function renderFooter(cfg, w, doc) {
  if (!cfg || !cfg.id) return;
  const el = doc.getElementById(cfg.id);
  if (!el) return;
  let text;
  if (cfg.uuidFallbackName) {
    // Destroyer: an unnamed lock reads as its raw uuid behind the marker glyph.
    const name = w.target_name || w.target_uuid || null;
    text = name ? ((cfg.prefix || '') + name) : t('console.common.no_target');
  } else {
    // Battleship / cruiser: an unnamed lock reads as the localized "LOCKED".
    text = w.target_name || (w.target_uuid ? t('console.common.locked') : t('console.common.no_target'));
    if (cfg.prefix) text = cfg.prefix + text;
  }
  el.textContent = text;
  if (cfg.colorize) {
    el.style.color = w.target_uuid ? 'var(--fire-bright)' : 'var(--ink-faint)';
  }
}
