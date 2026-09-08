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
 * stays a static "NO TARGET"), and a bespoke tail: the contextual Dock
 * control on the two hulls whose Helm owns a `dock` System (destroyer
 * #1164 S11a, cruiser #1388) plus the under-tow-load banner on the two
 * hulls that mount a tractor (destroyer #1157, cruiser #1390). Both halves
 * of that tail are `renderDockPanel` and `renderTowLoadPanel` below —
 * shared, not copied, so the two hulls cannot drift apart.
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
 *   Bespoke per-hull rendering the shared core does not cover (the contextual
 *   Dock control, the under-tow-load banner), called with `(s, doc, t)` after
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

    // ── AUTO badge ──────────────────────────────────────────────────────
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
 * The contextual dock control (issues #1159, #1388), shared by every hull whose
 * Helm owns a `kind = "dock"` System — the destroyer since #1164 S11a and the
 * cruiser since #1388. It lives here rather than in either hull's variant
 * because it is ONE control reading ONE authoritative view: a second copy would
 * be a second place for the dock/undock decision to drift.
 *
 * Hidden entirely unless the payload carries a dock view that is available,
 * engaged or docked, so a hull with no dock system — or one nowhere near a
 * berth — shows nothing at all. The button carries the authored System id in
 * `data-system-id` for the semantic adapter, toggles its label and `docked`
 * class off the server's own `docked` flag rather than re-deriving one, and
 * every string it writes is a `strings.csv` id through `t()` — no English
 * crosses here.
 *
 * A hull opts in by naming this as (or calling it from) its variant `tail`; its
 * markup supplies `dock-panel` / `dock-btn` / `dock-status` / `dock-refusal`.
 *
 * @param {object} s        the Helm console payload
 * @param {Document} doc
 * @param {function} tr     the string resolver (the shared `t`)
 */
export function renderDockPanel(s, doc, tr) {
  const dockPanel = doc.getElementById('dock-panel');
  const d = s.dock || null;
  const dockBtn = doc.getElementById('dock-btn');
  if (dockBtn) dockBtn.dataset.systemId = d?.system_id || '';
  if (!dockPanel) return;
  if (!d || (!d.available && !d.engaged && !d.docked)) {
    dockPanel.hidden = true;
    return;
  }
  dockPanel.hidden = false;
  const docked = !!d.docked;
  if (dockBtn) {
    dockBtn.classList.toggle('docked', docked);
    dockBtn.textContent = tr(docked ? 'console.dock.undock' : 'console.dock.dock');
  }
  const dockStatus = doc.getElementById('dock-status');
  if (dockStatus) {
    dockStatus.textContent = docked
      ? tr('console.dock.docked') + (d.docked_to_name ? ' · ' + tr(d.docked_to_name) : '')
      : tr('console.dock.available') + (d.available_target_name ? ' · ' + tr(d.available_target_name) : '');
  }
  const dockRefusal = doc.getElementById('dock-refusal');
  if (dockRefusal) {
    if (d.refusal) { dockRefusal.hidden = false; dockRefusal.textContent = tr(d.refusal); }
    else { dockRefusal.hidden = true; dockRefusal.textContent = ''; }
  }
}

/**
 * The under-tow-load banner (issues #1157, #1390), shared by every hull that
 * mounts a tractor — the destroyer since #1157 and the cruiser since #1390.
 * Shared here for `renderDockPanel`'s reason: one banner reading one
 * authoritative view.
 *
 * The tractor is ENGINEERING's control on both hulls, but the tow's mass
 * penalty lands on the HELM, so this seat is where the load has to be said.
 * `s.tow_load` is built by `buildHelmTowLoadView` in gui/console-state.js off
 * the same `tractor` blackboard the Engineering panel reads; a hull with no
 * tractor publishes none, so the view is null and the banner stays hidden.
 *
 * A hull opts in by calling this from its variant `tail`; its markup supplies
 * `tow-load-panel` / `tow-load-target`.
 *
 * @param {object} s      the Helm console payload
 * @param {Document} doc
 * @param {function} tr   the string resolver (the shared `t`)
 */
export function renderTowLoadPanel(s, doc, tr) {
  const towPanel = doc.getElementById('tow-load-panel');
  if (!towPanel) return;
  const tl = s.tow_load || null;
  if (!tl || !tl.active) {
    towPanel.hidden = true;
    return;
  }
  towPanel.hidden = false;
  const towTarget = doc.getElementById('tow-load-target');
  if (towTarget) towTarget.textContent = tl.target_name ? '· ' + tr(tl.target_name) : '';
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
