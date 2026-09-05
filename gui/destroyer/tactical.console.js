/**
 * gui/destroyer/tactical.console.js — the alliance destroyer's Tactical seat
 * (issue #1234).
 *
 * A system-id-keyed hull: the weapons panels read whichever weapons system the
 * seat actually owns, via projected Console Family metadata. Ship pose comes from that SAME weapons
 * view (like every other hull) — never from a navigation view. Adds a bespoke
 * tail for the Intel dossier (issue #1030) and the non-binding Command-intent
 * advice (issue #1108); the Intel overlay's own toggle wiring stays in the
 * `.html` via `initConsoleOverlays`.
 *
 * Since issue #1373 the two overlays are also TABS on the shell's Station Bar,
 * and this file is where Intel's unread count is worked out — only the console
 * knows whether the seat is looking at the panel.
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
    blasters: 'blasters-controls',
    torpedo: 'torpedo-controls',
    autoBadge: 'tactical-auto-badge',
  },
  // The destroyer authored a blaster column — it stays in view even while empty.
  torpedoMaxDefault: 0,
  footer: { id: 'footer-target', colorize: true, uuidFallbackName: true, prefix: '◉ ' },
  tail: (s, w, doc, t) => {
    // Intelligence files (issue #1030). Server-projected — nothing to filter
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

    // Security teams (issue #1346). Tactical OWNS the Security System on this
    // hull, so its view arrives under this station's payload — but reached
    // through the Security console family rather than a station-role key,
    // exactly as the engineering console reaches the tractor and the umbilical.
    // Another hull may give the same system to Command or Engineering and this
    // line is the only thing that would move. Nothing is filtered or re-derived
    // here: the panel renders what the server published, including which targets
    // are in reach.
    const securityEl = doc.getElementById('security-teams');
    if (securityEl) securityEl.state = familyView(s, 'security');

    // Non-binding Command intent advice (issue #1108): present only while
    // Command directs this Station and a human holds it. The label is a
    // strings id resolved here; falls back to the raw stance id.
    const adviceEl = doc.getElementById('command-advice');
    if (adviceEl) {
      const advice = s.command_advice || null;
      if (advice) {
        const stanceEl = doc.getElementById('command-advice-stance');
        if (stanceEl) {
          stanceEl.textContent = advice.stance_label ? t(advice.stance_label) : (advice.stance_id || '');
        }
        adviceEl.hidden = false;
      } else {
        adviceEl.hidden = true;
      }
    }
  },
});
