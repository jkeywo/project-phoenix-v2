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
 */
import { makeTacticalRender } from '../stations/tactical-console.js';
import { familyView } from '../console-payload.js';

export const renderStation = makeTacticalRender({
  weaponsView: (s) => familyView(s, 'tactical'),
  ids: {
    radar: 'tactical-radar',
    phasers: 'phasers-controls',
    blasters: 'blasters-controls',
    torpedo: 'torpedo-controls',
    stationDamage: 'station-damage',
    autoBadge: 'tactical-auto-badge',
  },
  // The destroyer authored a blaster column — it stays in view even while empty.
  torpedoMaxDefault: 0,
  footer: { id: 'footer-target', colorize: true, uuidFallbackName: true, prefix: '◉ ' },
  tail: (s, w, doc, t) => {
    // Intelligence files (issue #1030). Server-projected — nothing to filter
    // here — and independent of who currently hosts Comms (issue #1098).
    const dossierEl = doc.getElementById('dossier-panel');
    if (dossierEl) dossierEl.state = { dossiers: s.dossiers || [] };

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
