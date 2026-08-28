/**
 * gui/stations/helm-console.js — one Helm renderer, three hulls (issue
 * #1235, T4.C3 chunk 2 of the console-seam programme).
 *
 * The battleship, cruiser and alliance-destroyer Helm seats each shipped
 * their own inline `render(s)` in their `helm.html`. The three share a
 * radar + joystick + impulse/boost pair + station-damage footer + AUTO
 * badge core, differing in: whether the hull mounts a lateral-thrust
 * joystick (cruiser, destroyer; not battleship), how the target-contact
 * footer text is built (or whether it is touched at all — the destroyer's
 * stays a static "NO TARGET"), and a bespoke tail (the destroyer's
 * contextual Dock control and under-tow-load banner).
 *
 * Helm is a FLAT single-family payload — every hull calls `initConsole`
 * as an authoritative flat Helm-family payload — so `renderStation` reads
 * `s`'s fields directly.
 *
 * A hull supplies a `variant` object (below) and gets back a
 * `renderStation(s, doc = document)` it hands straight to `initConsole`'s
 * `render`.
 *
 * @typedef {object} HelmVariant
 * @property {object} ids                     element ids present in this hull's markup
 * @property {string} ids.radar                `ph-helm-radar` id (always present)
 * @property {string} ids.joystick             `ph-helm-joystick` id (always present)
 * @property {string} [ids.lateral]            `ph-lateral-thrust-joystick` id, if the hull
 *   mounts one (cruiser, destroyer; not battleship)
 * @property {string} ids.impulse              `ph-impulse-btn` id
 * @property {string} ids.boost                `ph-boost-btn` id
 * @property {string} [ids.stationDamage]      `ph-station-damage` id
 * @property {string} [ids.autoBadge]          the AUTO badge id
 * @property {object} [footer]                 target-contact footer config; omit for a hull
 *   whose footer text is static (the destroyer pattern — render never touches it)
 * @property {string} footer.id                the footer element id
 * @property {boolean} [footer.zeroFallback]   show the localized "no target" string at
 *   zero contacts instead of "0 contacts" (the cruiser pattern)
 * @property {boolean} [footer.glyph]          prefix a ◉ glyph when contacts > 0 (paired
 *   with zeroFallback — the cruiser pattern)
 * @property {boolean} [footer.colorize]       tint the footer text by contact count
 *   (paired with zeroFallback — the cruiser pattern)
 * @property {function(object, Document, function): void} [tail]
 *   Bespoke per-hull rendering the shared core does not cover (the destroyer's
 *   Dock control and under-tow-load banner), called with `(s, doc, t)` after
 *   the common panels are set.
 */

import { t } from '../strings.js';
import { setAutoState } from '../console-ui.js';

/**
 * Build a Helm `renderStation(s, doc)` for one hull from its `variant`.
 *
 * @param {HelmVariant} variant
 * @returns {function(object, Document=): void} renderStation
 */
export function makeHelmRender(variant) {
  const ids = variant.ids || {};

  return function renderStation(s, doc) {
    doc = doc || (typeof document !== 'undefined' ? document : null);
    if (!doc || !s) return;

    // ── Radar ────────────────────────────────────────────────────────────
    const radarEl = doc.getElementById(ids.radar);
    if (radarEl) {
      radarEl.state = {
        blips: s.blips || [], range: s.range || 500, x: s.x || 0, z: s.z || 0,
        ship_heading: s.ship_heading || 0, speed: s.speed || 0, on_screen_active: !!s.on_screen,
        config: {}, engine_port_thrust: s.engine_port_thrust || 0, engine_stbd_thrust: s.engine_stbd_thrust || 0,
        hostile_arcs: s.hostile_arcs || [], hostile_arc_color: s.hostile_arc_color || null,
      };
    }

    // ── Joystick(s) ──────────────────────────────────────────────────────
    const joystickEl = doc.getElementById(ids.joystick);
    if (joystickEl) joystickEl.state = { auto: !!s.helm_auto };
    if (ids.lateral) {
      const lateralEl = doc.getElementById(ids.lateral);
      if (lateralEl) lateralEl.state = { auto: !!s.lateral_auto };
    }

    // ── Impulse / boost ──────────────────────────────────────────────────
    const impulseEl = doc.getElementById(ids.impulse);
    if (impulseEl) {
      impulseEl.state = {
        state: s.impulse_charge_progress > 0 ? 'charging' : 'ready',
        charge_pct: (s.impulse_charge_progress || 0) * 100,
        auto: !!s.helm_auto,
      };
    }
    const boostEl = doc.getElementById(ids.boost);
    if (boostEl) {
      boostEl.state = {
        available: !!s.boost_enabled, active: !!s.boost_active,
        recharge_pct: s.boost_battery != null ? s.boost_battery * 100 : 100,
        auto: !!s.helm_auto,
      };
    }

    // ── Station-damage + AUTO badge ─────────────────────────────────────
    if (ids.stationDamage) {
      const el = doc.getElementById(ids.stationDamage);
      if (el) el.state = s.own_hull || null;
    }
    if (ids.autoBadge) {
      const el = doc.getElementById(ids.autoBadge);
      if (el) setAutoState(null, el, !!s.helm_auto);
    }

    // ── Target-contact footer ───────────────────────────────────────────
    renderContactFooter(variant.footer, s, doc);

    // ── Bespoke per-hull tail ────────────────────────────────────────────
    if (variant.tail) variant.tail(s, doc, t);
  };
}

/**
 * The shared target-contact footer. Battleship and cruiser each carry one
 * (with different fallback/tint rules); the destroyer's is static — its
 * variant carries no `footer` config, so this is a no-op for it.
 *
 * @param {object|undefined} cfg  the variant's `footer` config, or undefined
 * @param {object} s              the (shape-normalised) console payload
 * @param {Document} doc
 */
function renderContactFooter(cfg, s, doc) {
  if (!cfg || !cfg.id) return;
  const el = doc.getElementById(cfg.id);
  if (!el) return;
  const n = (s.blips || []).length;
  if (cfg.zeroFallback) {
    el.textContent = n > 0
      ? (cfg.glyph ? '◉ ' : '') + (n === 1 ? t('console.common.contacts.one', { n }) : t('console.common.contacts.other', { n }))
      : t('console.common.no_target');
    if (cfg.colorize) el.style.color = n > 0 ? 'var(--ink-dim)' : 'var(--ink-faint)';
  } else {
    el.textContent = n === 1 ? t('console.common.contacts.one', { n: 1 }) : t('console.common.contacts.other', { n });
  }
}
