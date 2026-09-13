import { t } from './strings.js';

/** Live presentation only: one current label for each informative sound family,
 * overwritten/coalesced in place and removed after its live indication. No list,
 * navigation, replay, localStorage or transfer into Comms/mission history. */
export function createAudioLiveEquivalents(doc) {
  const host = doc.createElement('div');
  host.id = 'audio-live-equivalents';
  host.className = 'audio-live-equivalents';
  host.setAttribute('aria-live', 'polite');
  host.setAttribute('aria-atomic', 'false');
  doc.body.append(host);
  const rows = new Map();
  const timers = new Map();
  function clear(kind) {
    clearTimeout(timers.get(kind));
    timers.delete(kind);
    rows.get(kind)?.remove();
    rows.delete(kind);
  }
  function show(kind, label, transient = false) {
    let row = rows.get(kind);
    if (!row) {
      row = doc.createElement('div'); row.dataset.audioEquivalent = kind;
      rows.set(kind, row); host.append(row);
    }
    if (row.textContent !== label) row.textContent = label;
    if (transient) {
      clearTimeout(timers.get(kind));
      // [ai] A bounded live indication, not a review/catch-up window. Repeated
      // combat cues replace the same label; they cannot accumulate in the DOM.
      timers.set(kind, setTimeout(() => clear(kind), 2000));
    }
  }
  function update(cue) {
    if (cue.kind === 'clear') { for (const key of [...rows.keys()]) clear(key); }
    if (cue.kind === 'beam') {
      if (cue.active) show('beam', t('audio.cue.beam'));
      else clear('beam');
    }
    if (cue.kind === 'impact') show('impact', t('audio.cue.impact'), true);
    if (cue.kind === 'blaster') {
      const bearing = Math.round((Math.atan2(cue.x, -cue.z) * 180 / Math.PI + 360) % 360) % 360;
      const elevation = Math.round(Math.atan2(cue.y, Math.hypot(cue.x, cue.z)) * 180 / Math.PI);
      show('blaster', t('audio.cue.blaster', { bearing, elevation }), true);
    }
  }
  return { update, dispose() { update({ kind: 'clear' }); host.remove(); } };
}
