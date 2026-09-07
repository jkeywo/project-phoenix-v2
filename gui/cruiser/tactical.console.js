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
 */
import { makeTacticalRender } from '../stations/tactical-console.js';
import { familyView } from '../console-payload.js';

export const renderStation = makeTacticalRender({
  weaponsView: (s) => familyView(s, 'tactical'),
  ids: {
    radar: 'tactical-radar',
    phasers: 'phasers-controls',
    torpedo: 'torpedo-controls',
    autoBadge: 'tactical-auto-badge',
  },
  torpedoMaxDefault: 20,
  footer: { id: 'footer-target', colorize: true },
  tail: (s, w, doc) => {
    // Security teams (issue #1389). Tactical OWNS the Security System on this
    // hull, so its view arrives under this station's payload — but reached
    // through the Security console family rather than a station-role key,
    // exactly as the destroyer's Tactical seat reaches it. A hull that gave the
    // same system to Command or Engineering would move this line and nothing
    // else. Nothing is filtered or re-derived here: the panel renders what the
    // server published, including which targets are in reach.
    const securityEl = doc.getElementById('security-teams');
    if (securityEl) securityEl.state = familyView(s, 'security');
  },
});
