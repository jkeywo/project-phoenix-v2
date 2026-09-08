/**
 * gui/cruiser/tactical.console.js — the cruiser's Tactical seat (issue #1234).
 *
 * Like the battleship (phasers + torpedoes) but with no blaster mount and a
 * colorized target footer. This rewrite also eliminated the old inline bug:
 * `gui/cruiser/tactical.html` used to do
 * `var t = document.getElementById('torpedo-controls')`, shadowing the imported
 * String Table `t()` — so `t('console.common.locked')` threw when a locked
 * target had no name. The shared renderer owns the footer now, using the real
 * `t`; nothing here shadows it.
 *
 * SINCE ISSUE #1389 THIS SEAT IS SYSTEM-ID-KEYED, not flat. The hull authors a
 * Security System on Tactical (`alliance_cruiser.toml`), so the station's owned
 * systems span two Console Families and `buildConsoleStateInner` gives it the
 * generic keyed payload instead of the flat single-family one. The weapons
 * panels therefore read through `familyView` like the destroyer's, which is
 * correct for BOTH shapes: a flat payload is normalised to the keyed one at the
 * console-core seam (`normalizeConsolePayload`) before `render` ever sees it.
 *
 * Issue #1393 adds the target lock card (below the phaser rail, fed
 * generically by `renderTargetCard` once `ids.targetCard` is present — see
 * `gui/stations/tactical-console.js`) and the Intel overlay tail, mirroring
 * the destroyer's `gui/destroyer/tactical.console.js`. `dossiers` already
 * rides every system-keyed payload (issue #1378), including the cruiser's, so
 * nothing upstream of this file needed to change.
 */
import { makeTacticalRender } from '../stations/tactical-console.js';
import { familyView } from '../console-payload.js';
import { openConsoleOverlayId } from '../console-overlays.js';
import { intelUnreadCount, markIntelSeen } from '../intel-unread.js';

/**
 * How much Intel this seat had already read, last time it looked (issue
 * #1373). Held per DOCUMENT rather than in a module variable: the baseline is
 * a fact about the console the player is sitting at, not about this module, so
 * a second document (a test's fresh jsdom, a reloaded iframe) starts from an
 * empty baseline the way a player who has never opened the panel does.
 */
const intelSeenByDocument = new WeakMap();

export const renderStation = makeTacticalRender({
  weaponsView: (s) => familyView(s, 'tactical'),
  ids: {
    radar: 'tactical-radar',
    phasers: 'phasers-controls',
    torpedo: 'torpedo-controls',
    autoBadge: 'tactical-auto-badge',
    // Target lock card (issue #1393): name/stance/class/bearing/range/hull/
    // shield facings/shield frequency, below the phaser column — the same
    // generic card the destroyer mounts.
    targetCard: 'target-lock-card',
  },
  torpedoMaxDefault: 20,
  footer: { id: 'footer-target', colorize: true },
  tail: (s, w, doc, t) => {
    // Security teams (issue #1389). Tactical OWNS the Security System on this
    // hull, so its published view arrives under this station's payload — but
    // reached through the Security console family rather than a station-role
    // key, exactly as the destroyer's Tactical seat reaches it. A hull that gave
    // the same system to Command or Engineering would move this line and
    // nothing else. Nothing is filtered or re-derived here: the panel renders
    // what the server published, including which targets are in reach.
    const securityEl = doc.getElementById('security-teams');
    if (securityEl) securityEl.state = familyView(s, 'security');

    // Intelligence files (issue #1393). Server-projected — nothing to filter
    // here — and independent of who currently hosts Comms (issue #1098).
    const dossierEl = doc.getElementById('dossier-panel');
    const dossiers = s.dossiers || [];
    if (dossierEl) dossierEl.state = { dossiers };

    // The Intel tab's unread badge (issue #1373). Reading is what clears it, so
    // the baseline is taken while the panel is actually OPEN — console-core
    // re-renders the moment the bar opens one, so the badge clears on the tap
    // rather than on the next state push. A count that has not moved re-posts
    // nothing (see `__setConsoleTabBadge`), so this runs on every render for
    // free.
    const intelPanelOpen = openConsoleOverlayId(doc) === 'intel-overlay';
    let seen = intelSeenByDocument.get(doc);
    if (intelPanelOpen) {
      seen = markIntelSeen(dossiers, seen);
      intelSeenByDocument.set(doc, seen);
    }
    const setBadge = typeof window !== 'undefined' && window.__setConsoleTabBadge;
    if (typeof setBadge === 'function') {
      setBadge('intel-overlay', intelUnreadCount(dossiers, seen));
    }
  },
});
