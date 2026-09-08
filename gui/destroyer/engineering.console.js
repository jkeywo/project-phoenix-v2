/**
 * gui/destroyer/engineering.console.js — the alliance destroyer's Engineering
 * seat (issue #1235).
 *
 * The Shields + Power + Repair core, plus a bespoke tail of three panels:
 * Tractor beam (issue #1156), Transfer umbilical (issue #1160) and External
 * repair-team dispatch (issue #1161, `[repair.external_dispatch]`). Each panel
 * hides itself entirely when this hull's payload carries no matching system
 * view, so a hull without one stays unchanged.
 *
 * The first two are SHARED with the cruiser, which mounts the same pair since
 * #1390: their bodies are `renderTractorPanel` / `renderUmbilicalPanel` in
 * gui/stations/engineering-console.js rather than a copy here, so the two hulls
 * cannot drift apart. The dispatch panel stays local — this is the only player
 * hull that authors `[repair.external_dispatch]`.
 *
 * Every panel here toggles one button between two admitted commands (engage/
 * release, start/stop, dispatch/recall) — human and AI both issue the SAME
 * command through the one admission path, this control just picks which. The
 * click handlers live in `engineering.html` (they call `sendAction`, which
 * only exists after `initConsole` runs) and read the "which one" off the
 * button's own `.engaged` class rather than a variable threaded back out of
 * `render` — the class is set on every render, so it is never stale.
 */
import {
  makeEngineeringRender,
  renderTractorPanel,
  renderUmbilicalPanel,
} from '../stations/engineering-console.js';

export const renderStation = makeEngineeringRender({
  ids: {
    shieldFacings: 'shield-facings',
    threatRow: 'threat-row',
    threatBearing: 'threat-bearing',
    power: 'power-controls',
    battery: 'battery-bar',
    hullIntegrity: 'hull-integrity',
    coreDamage: 'core-damage',
    repairTeams: 'repair-teams',
    autoBadge: 'engineering-auto-badge',
  },
  tail: (s, views, doc, t) => {
    // ── Operations: tractor beam (#1156) and transfer umbilical (#1160) ──
    renderTractorPanel(s, doc, t);
    renderUmbilicalPanel(s, doc, t);

    // ── External repair-team dispatch (issue #1161) ────────────────────
    // On this hull the repair system is engineering-owned, so the dispatch
    // view rides `views.repair` (the same payload the battleship's dedicated
    // Repair console reads off its own flat payload).
    const dPanel = doc.getElementById('dispatch-panel');
    const ed = views.repair.external_dispatch || null;
    if (dPanel) {
      if (!ed) {
        dPanel.hidden = true;
      } else {
        dPanel.hidden = false;
        const working = ed.target != null;
        const dBtn = doc.getElementById('dispatch-btn');
        if (dBtn) {
          dBtn.classList.toggle('engaged', working);
          dBtn.textContent = t(working ? 'console.repair.dispatch.recall' : 'console.repair.dispatch.send');
        }
        const dStatus = doc.getElementById('dispatch-status');
        if (dStatus) {
          dStatus.textContent = working
            ? t('console.repair.dispatch.working') + (ed.target_name ? ' · ' + t(ed.target_name) : '')
            : t('console.repair.dispatch.idle') + ' · ' + t('console.repair.dispatch.range') + ' ' + Math.round(ed.range || 0);
        }
        const dRefusal = doc.getElementById('dispatch-refusal');
        if (dRefusal) {
          if (ed.refusal) { dRefusal.hidden = false; dRefusal.textContent = t(ed.refusal); }
          else { dRefusal.hidden = true; dRefusal.textContent = ''; }
        }
      }
    }
  },
});
