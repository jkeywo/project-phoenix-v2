// @vitest-environment jsdom
//
// tests/client/page-chrome.test.js — issue #1227: server.html and
// client.html both carried verbatim copies of the connection-status dot,
// the Screen Wake Lock (+ visibilitychange re-acquire), the fullscreen
// toggle, and the mechanical half of the #conn-diag renderer. This suite
// exercises gui/page-chrome.js directly rather than scraping either page's
// source text — same rationale as host-channel-localisation.test.js for
// gui/host-channel.js (issue #1225).
//
// The two pages' actual differences (hex vs CSS-var colours, differing
// string ids, client-only retry button, server-only beforeunload release)
// are covered here as the mount-time options that carry them, per
// gui/page-chrome.js's own doc comment.
//
// Each test mounts onto a FRESH Document (document.implementation.
// createHTMLDocument), not the shared jsdom global `document`: mount
// installs a document-level `visibilitychange`/`fullscreenchange` listener
// with no matching unmount, and the global document is shared across every
// test in this file (see focus-trap.test.js for the same jsdom caveat) — a
// fresh document per test is what actually isolates them, rather than
// tracking and releasing listeners by hand.

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { mountPageChrome } from '../../gui/page-chrome.js';

/** A `t` stub that renders id + params rather than looking anything up. */
function fakeT(id, params) {
  return params ? `${id}:${JSON.stringify(params)}` : id;
}

/** A fresh, isolated Document with the elements chrome needs. Any element
 * can be omitted per test. */
function scene({ retryBtn = true, fullscreenBtn = true, connDiag = true } = {}) {
  const doc = document.implementation.createHTMLDocument('page-chrome test');
  doc.body.innerHTML = `
    <span id="conn-label"></span>
    <span id="conn-dot"></span>
    ${retryBtn ? '<button id="retry-now-btn">Retry now</button>' : ''}
    ${fullscreenBtn ? '<button id="fullscreen-btn">⛶</button>' : ''}
    ${connDiag ? '<div id="conn-diag"></div>' : ''}
  `;
  Object.defineProperty(doc, 'fullscreenElement', {
    value: null, writable: true, configurable: true,
  });
  doc.documentElement.requestFullscreen = vi.fn(() => Promise.resolve());
  doc.exitFullscreen = vi.fn(() => Promise.resolve());
  return doc;
}

/** A fake WakeLockSentinel: records release() calls and lets a test fire 'release'. */
function fakeSentinel() {
  const listeners = {};
  return {
    released: false,
    release: vi.fn(function () {
      this.released = true;
      (listeners.release || []).forEach((fn) => fn());
    }),
    addEventListener: vi.fn((event, fn) => {
      (listeners[event] = listeners[event] || []).push(fn);
    }),
  };
}

describe('mountPageChrome', () => {
  let doc;
  let requestWakeLock;

  beforeEach(() => {
    doc = scene();
    requestWakeLock = vi.fn(() => Promise.resolve(fakeSentinel()));
    Object.defineProperty(navigator, 'wakeLock', {
      value: { request: requestWakeLock },
      configurable: true,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  // ── Connection status dot ────────────────────────────────────────────

  describe('setConnectionStatus', () => {
    it('defaults to server.html’s original hex colours and string ids', () => {
      const chrome = mountPageChrome({ doc, t: fakeT });
      chrome.setConnectionStatus('disconnected');
      expect(doc.getElementById('conn-dot').style.background).toBe('rgb(255, 68, 68)');
      expect(doc.getElementById('conn-label').textContent).toBe('client.reconnecting');

      chrome.setConnectionStatus('error');
      expect(doc.getElementById('conn-label').textContent).toBe('server.conn_error_refresh');

      chrome.setConnectionStatus('ready');
      expect(doc.getElementById('conn-dot').style.background).toBe('rgb(68, 204, 68)');
      expect(doc.getElementById('conn-label').textContent).toBe('');
    });

    it('honours per-page colours and labels (client.html’s shape)', () => {
      const chrome = mountPageChrome({
        doc,
        t: fakeT,
        statusColors: { active: 'var(--loaded)', trouble: 'var(--fire-hot)' },
        statusLabels: { disconnected: 'client.reconnecting', error: 'client.conn_error' },
      });
      chrome.setConnectionStatus('error');
      expect(doc.getElementById('conn-dot').style.background).toBe('var(--fire-hot)');
      expect(doc.getElementById('conn-label').textContent).toBe('client.conn_error');
    });

    it('shows the retry button on disconnected/error and hides it otherwise', () => {
      const chrome = mountPageChrome({ doc, t: fakeT });
      const retryBtn = doc.getElementById('retry-now-btn');

      chrome.setConnectionStatus('disconnected');
      expect(retryBtn.classList.contains('visible')).toBe(true);

      chrome.setConnectionStatus('error');
      expect(retryBtn.classList.contains('visible')).toBe(true);

      chrome.setConnectionStatus('ready');
      expect(retryBtn.classList.contains('visible')).toBe(false);
    });

    it('is a no-op for the retry button when the page has none (server.html)', () => {
      doc = scene({ retryBtn: false });
      const chrome = mountPageChrome({ doc, t: fakeT });
      expect(() => chrome.setConnectionStatus('disconnected')).not.toThrow();
    });

    it('does nothing if the dot/label are missing rather than throwing', () => {
      doc.body.innerHTML = '';
      const chrome = mountPageChrome({ doc, t: fakeT });
      expect(() => chrome.setConnectionStatus('error')).not.toThrow();
    });

    it('treats every non-disconnected/error state (connecting, ready, …) the same', () => {
      const chrome = mountPageChrome({ doc, t: fakeT });
      for (const state of ['connecting', 'ready', 'anything-else']) {
        chrome.setConnectionStatus(state);
        expect(doc.getElementById('conn-dot').style.background).toBe('rgb(68, 204, 68)');
        expect(doc.getElementById('conn-label').textContent).toBe('');
      }
    });
  });

  // ── Screen Wake Lock ──────────────────────────────────────────────────

  describe('wake lock', () => {
    it('requests the screen wake lock once, deduping a second concurrent acquire', async () => {
      const chrome = mountPageChrome({ doc, t: fakeT });
      await chrome.acquireWakeLock();
      await chrome.acquireWakeLock();
      expect(requestWakeLock).toHaveBeenCalledTimes(1);
      expect(requestWakeLock).toHaveBeenCalledWith('screen');
    });

    it('releases the held sentinel and allows re-acquiring afterwards', async () => {
      const chrome = mountPageChrome({ doc, t: fakeT });
      await chrome.acquireWakeLock();
      chrome.releaseWakeLock();
      await chrome.acquireWakeLock();
      expect(requestWakeLock).toHaveBeenCalledTimes(2);
    });

    it('swallows a rejected wakeLock.request (e.g. unsupported/denied)', async () => {
      Object.defineProperty(navigator, 'wakeLock', {
        value: { request: vi.fn(() => Promise.reject(new Error('nope'))) },
        configurable: true,
      });
      const chrome = mountPageChrome({ doc, t: fakeT });
      await expect(chrome.acquireWakeLock()).resolves.toBeUndefined();
    });

    it('re-acquires on visibilitychange when the document becomes visible', () => {
      mountPageChrome({ doc, t: fakeT });
      Object.defineProperty(doc, 'visibilityState', { value: 'visible', configurable: true });
      doc.dispatchEvent(new Event('visibilitychange'));
      expect(requestWakeLock).toHaveBeenCalledTimes(1);
    });

    it('does not acquire on visibilitychange while hidden', () => {
      mountPageChrome({ doc, t: fakeT });
      Object.defineProperty(doc, 'visibilityState', { value: 'hidden', configurable: true });
      doc.dispatchEvent(new Event('visibilitychange'));
      expect(requestWakeLock).not.toHaveBeenCalled();
    });

    it('releases on beforeunload only when releaseWakeLockOnUnload opts in (server.html)', async () => {
      const chrome = mountPageChrome({ doc, t: fakeT, releaseWakeLockOnUnload: true });
      await chrome.acquireWakeLock();
      window.dispatchEvent(new Event('beforeunload'));
      // releaseWakeLock clears the sentinel; a fresh acquire proves it ran.
      await chrome.acquireWakeLock();
      expect(requestWakeLock).toHaveBeenCalledTimes(2);
    });

    it('does not wire beforeunload when the option is left off (client.html)', async () => {
      const chrome = mountPageChrome({ doc, t: fakeT }); // default: false
      await chrome.acquireWakeLock();
      window.dispatchEvent(new Event('beforeunload'));
      await chrome.acquireWakeLock();
      // Still held from the first acquire — beforeunload didn't release it —
      // so the second acquire is a deduped no-op.
      expect(requestWakeLock).toHaveBeenCalledTimes(1);
    });
  });

  // ── Fullscreen controller ────────────────────────────────────────────

  describe('fullscreen', () => {
    it('requests fullscreen on click when not already fullscreen', () => {
      mountPageChrome({ doc, t: fakeT });
      doc.getElementById('fullscreen-btn').click();
      expect(doc.documentElement.requestFullscreen).toHaveBeenCalledTimes(1);
      expect(doc.exitFullscreen).not.toHaveBeenCalled();
    });

    it('exits fullscreen on click when already fullscreen', () => {
      Object.defineProperty(doc, 'fullscreenElement', {
        value: doc.body, writable: true, configurable: true,
      });
      mountPageChrome({ doc, t: fakeT });
      doc.getElementById('fullscreen-btn').click();
      expect(doc.exitFullscreen).toHaveBeenCalledTimes(1);
      expect(doc.documentElement.requestFullscreen).not.toHaveBeenCalled();
    });

    it('syncs the button icon on fullscreenchange', () => {
      mountPageChrome({ doc, t: fakeT });
      const btn = doc.getElementById('fullscreen-btn');
      expect(btn.textContent).toBe('⛶'); // unchanged until an event fires

      Object.defineProperty(doc, 'fullscreenElement', {
        value: doc.body, writable: true, configurable: true,
      });
      doc.dispatchEvent(new Event('fullscreenchange'));
      expect(btn.textContent).toBe('✕');

      Object.defineProperty(doc, 'fullscreenElement', {
        value: null, writable: true, configurable: true,
      });
      doc.dispatchEvent(new Event('fullscreenchange'));
      expect(btn.textContent).toBe('⛶');
    });

    it('does not throw when the page has no fullscreen button', () => {
      doc = scene({ fullscreenBtn: false });
      expect(() => mountPageChrome({ doc, t: fakeT })).not.toThrow();
    });
  });

  // ── Connection diagnostics (mechanical half) ─────────────────────────

  describe('mountConnDiag', () => {
    it('joins lines with newlines into the target element', () => {
      const chrome = mountPageChrome({ doc, t: fakeT });
      const render = chrome.mountConnDiag('conn-diag');
      render(['first line', 'second line']);
      expect(doc.getElementById('conn-diag').textContent).toBe('first line\nsecond line');
    });

    it('is a no-op when the element is missing, rather than throwing', () => {
      doc = scene({ connDiag: false });
      const chrome = mountPageChrome({ doc, t: fakeT });
      const render = chrome.mountConnDiag('conn-diag');
      expect(() => render(['x'])).not.toThrow();
    });

    it('supports a custom element id', () => {
      doc.body.insertAdjacentHTML('beforeend', '<div id="other-diag"></div>');
      const chrome = mountPageChrome({ doc, t: fakeT });
      const render = chrome.mountConnDiag('other-diag');
      render(['hi']);
      expect(doc.getElementById('other-diag').textContent).toBe('hi');
      expect(doc.getElementById('conn-diag').textContent).toBe('');
    });
  });
});
