/**
 * gui/settings-panel.js — In-game settings overlay for all client consoles.
 *
 * Provides a gear button (bottom-left of the host page) that opens a popup
 * with Rating selection, Volume slider, QR code toggle, and Leave Station.
 *
 * Usage in client.html:
 *
 *   <script type="module">
 *     import { mountSettings } from './gui/settings-panel.js';
 *     mountSettings({
 *       send: (type, data) => connectionManager.send(type, data, 'reliable'),
 *       getState: () => state,
 *       audioEl: document.getElementById('ui-click'),
 *     });
 *   </script>
 */

import { t, has } from './strings.js';

const STORAGE_KEY = 'phoenix-settings-volume';

/**
 * Mount the settings button + overlay on `doc`.
 *
 * @param {{ send: function, getState: function, audioEl: HTMLAudioElement|null }} opts
 * @returns {{ open: function, close: function, destroy: function }}
 */
export function mountSettings({ send, getState, audioEl, myToken, doc: _doc } = {}) {
  const doc = _doc || (typeof document !== 'undefined' ? document : null);
  if (!doc) return { open() {}, close() {}, rebuildContent() {} };

  // ── Gear button ──────────────────────────────────────────────────────────
  let btn = doc.getElementById('settings-btn');
  if (!btn) {
    btn = doc.createElement('button');
    btn.id = 'settings-btn';
    btn.className = 'settings-btn';
    btn.setAttribute('aria-label', t('settings.title'));
    btn.title = t('settings.title');
    btn.textContent = '\u2699';
    doc.body.appendChild(btn);
  }

  // ── Overlay ──────────────────────────────────────────────────────────────
  let overlay = doc.getElementById('settings-overlay');
  if (!overlay) {
    overlay = doc.createElement('div');
    overlay.id = 'settings-overlay';
    overlay.className = 'settings-overlay';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.setAttribute('aria-hidden', 'true');
    overlay.hidden = true;
    doc.body.appendChild(overlay);
  }

  // ── Render helpers ───────────────────────────────────────────────────────

  function buildContent() {
    overlay.innerHTML = '';
    // Wrap content in a .settings-popup div for styling.
    const popup = doc.createElement('div');
    popup.className = 'settings-popup';
    overlay.appendChild(popup);

    const s = getState ? getState() : {};
    const myStation = (s.stations || []).find(st => st.holder_token === myToken);
    const sid = myStation ? myStation.id : null;
    const ratings = (myStation && myStation.ratings) || [];
    const activeRating = (sid && s.stationRatings && s.stationRatings[sid]) || (ratings[0] || '');

    // ── Rating (complexity) section ────────────────────────────────────────
    if (sid && ratings.length > 1) {
      const ratingSection = doc.createElement('div');
      ratingSection.className = 'settings-section';

      const heading = doc.createElement('div');
      heading.className = 'settings-section-heading';
      heading.textContent = t('settings.rating');
      ratingSection.appendChild(heading);

      const row = doc.createElement('div');
      row.className = 'settings-rating-row';
      for (const r of ratings) {
        const btn2 = doc.createElement('button');
        btn2.className = 'settings-rating-btn' + (r === activeRating ? ' active' : '');
        // Rating names are lookup identifiers in the ship TOML (Rust matches
        // them by name), so the display text comes from a derived string id.
        const ratingKey = 'station.rating.' + r.toLowerCase() + '.name';
        btn2.textContent = has(ratingKey) ? t(ratingKey) : r.toUpperCase();
        btn2.addEventListener('click', () => {
          if (r !== activeRating && typeof send === 'function') {
            send('SetStationRating', { rating_name: r });
          }
        });
        row.appendChild(btn2);
      }
      ratingSection.appendChild(row);
      popup.appendChild(ratingSection);
    }

    // ── Volume section ─────────────────────────────────────────────────────
    const volSection = doc.createElement('div');
    volSection.className = 'settings-section';

    const volHeading = doc.createElement('div');
    volHeading.className = 'settings-section-heading';
    volHeading.textContent = t('settings.volume');
    volSection.appendChild(volHeading);

    const volRow = doc.createElement('div');
    volRow.className = 'settings-vol-row';

    const volSlider = doc.createElement('input');
    volSlider.type = 'range';
    volSlider.min = '0';
    volSlider.max = '1';
    volSlider.step = '0.05';
    var initialVol = 1.0;
    try { var stored = localStorage.getItem(STORAGE_KEY); if (stored !== null) initialVol = parseFloat(stored); } catch (_) {}
    volSlider.value = String(initialVol);
    if (audioEl) audioEl.volume = initialVol;
    volSlider.addEventListener('input', function () {
      var v = parseFloat(this.value);
      if (audioEl) audioEl.volume = v;
      try { localStorage.setItem(STORAGE_KEY, String(v)); } catch (_) {}
    });

    const volLabel = doc.createElement('span');
    volLabel.className = 'settings-vol-label';
    volLabel.textContent = Math.round(initialVol * 100) + '%';
    volSlider.addEventListener('input', function () {
      const pct = Math.round(parseFloat(this.value) * 100);
      volLabel.textContent = pct + '%';
    });

    volRow.appendChild(volSlider);
    volRow.appendChild(volLabel);
    volSection.appendChild(volRow);
    popup.appendChild(volSection);

    // ── QR Code section ───────────────────────────────────────────────────
    const qrSection = doc.createElement('div');
    qrSection.className = 'settings-section';

    const qrHeading = doc.createElement('div');
    qrHeading.className = 'settings-section-heading';
    qrHeading.textContent = t('settings.qr_code');
    qrSection.appendChild(qrHeading);

    const qrBtn = doc.createElement('button');
    qrBtn.className = 'settings-action-btn';
    qrBtn.textContent = t('settings.toggle_qr');
    qrBtn.addEventListener('click', () => {
      if (typeof send === 'function') send('ToggleQrCode', {});
    });
    qrSection.appendChild(qrBtn);
    popup.appendChild(qrSection);

    // ── Leave Station section ──────────────────────────────────────────────
    if (sid) {
      const leaveSection = doc.createElement('div');
      leaveSection.className = 'settings-section';

      const leaveHeading = doc.createElement('div');
      leaveHeading.className = 'settings-section-heading';
      leaveHeading.textContent = t('settings.station');
      leaveSection.appendChild(leaveHeading);

      const leaveBtn = doc.createElement('button');
      leaveBtn.className = 'settings-action-btn settings-leave-btn';
      leaveBtn.textContent = t('settings.leave_station');
      leaveBtn.addEventListener('click', () => {
        close();
        if (typeof send === 'function') send('ReleaseStation');
      });
      leaveSection.appendChild(leaveBtn);
      popup.appendChild(leaveSection);
    }
  }

  function open() {
    overlay.hidden = false;
    overlay.setAttribute('aria-hidden', 'false');
    overlay.classList.add('open');
    buildContent();
  }

  function close() {
    overlay.hidden = true;
    overlay.setAttribute('aria-hidden', 'true');
    overlay.classList.remove('open');
  }

  // ── Wire button ─────────────────────────────────────────────────────────−
  btn.addEventListener('click', (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (overlay.classList.contains('open')) {
      close();
    } else {
      open();
    }
  });

  // Dismiss on backdrop click.
  overlay.addEventListener('click', (e) => {
    if (e.target === overlay) close();
  });

  // ── Rebuild content on state changes (polled from render) ───────────────
  // Exposed so client.html can call rebuildContent() from its render loop
  // whenever state changes (so the active rating button stays in sync).

  return { open, close, rebuildContent: buildContent };
}

// Expose for non-module scripts (fallback).
if (typeof window !== 'undefined') {
  window.mountSettings = mountSettings;
}
