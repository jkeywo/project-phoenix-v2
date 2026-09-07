/**
 * gui/stations/comms-console.js — one Comms renderer, two hulls (issue
 * #1235, T4.C3 chunk 3 — final chunk of the console-seam programme).
 *
 * Only the battleship and cruiser mount a dedicated Comms Station (the
 * destroyer and courier have none — see the per-hull ship TOMLs). The two shipped their
 * own inline `render(s)` in `comms.html`. The shared core is the HAILS |
 * CONTACTS list pair, the open thread and the active-hail readout — every
 * hull mounts those with the same ids. The cruiser's Comms seat shares its
 * document with the cruiser's auxiliary Navigation Station, so its `tail`
 * also drives that view and decides which of the two is on screen; the
 * battleship has no tail at all.
 *
 * INBOX SHAPE (issue #1380). The list is one row per THREAD, not per message:
 * `sortedThreadsFrom` groups the wire messages, `threadMessagesFrom` supplies
 * the open thread's whole history, and the current-message panel renders that
 * history with the active message's responses pinned beneath it. Both
 * functions are pure and live in `gui/comms-state.js`, beside the store whose
 * own methods delegate to them.
 *
 * A hull supplies a `variant` object (below) and gets back a
 * `renderStation(s, doc = document)` it hands straight to `initConsole`'s
 * `render`. The same function is importable by a vitest suite.
 *
 * @typedef {object} CommsVariant
 * @property {function(object): object} [commsView]
 *   Given the (already shape-normalised) console payload, return the "view"
 *   the shared core reads contacts/messages fields from. The battleship
 *   omits this (flat `comms` family payload — the core defaults to `s`
 *   itself); the cruiser returns `familyView(s, 'comms')`.
 * @property {object} ids                     element ids present in this hull's markup
 * @property {string} [ids.contactList]       `ph-comms-contact-list` id
 * @property {string} [ids.hailList]          `ph-comms-hail-list` id
 * @property {string} [ids.currentMessage]    `ph-comms-current-message` id
 * @property {string} [ids.hailsUnread]       unread-thread count on the HAILS tab
 * @property {string} [ids.activeHail]        readout naming the open thread's channel
 * @property {string} [ids.threadPanel]       `.overlay-panel` holding the open thread
 * @property {string} [ids.autoBadge]         the AUTO badge id
 * @property {function(object, object): boolean} [autoState]
 *   Compute the AUTO badge state from `(s, view)`. Defaults to
 *   `!!view.comms_auto`. The cruiser conjuncts Navigation's own auto flag —
 *   see its variant.
 * @property {function(object, object, Document, function, object|null): void} [tail]
 *   Bespoke per-hull rendering the shared core does not cover, called with
 *   `(s, view, doc, t, activeMessage)` after the common panels are set.
 */

import { t } from '../strings.js';
import { setAutoState } from '../console-ui.js';
import { setConsoleOverlay } from '../console-overlays.js';
import {
  activeThreadMessageFrom,
  effectiveThreadId,
  isLatestLiveCriticalMessage,
  sortedThreadsFrom,
  threadMessagesFrom,
  unreadThreadCount,
} from '../comms-state.js';

/**
 * Build a Comms `renderStation(s, doc)` for one hull from its `variant`.
 *
 * @param {CommsVariant} variant
 * @returns {function(object, Document=): void} renderStation
 */
export function makeCommsRender(variant) {
  const ids = variant.ids || {};
  // Two presentation-state owners per console document, and they are two
  // because the surface asks two different questions. A HAILS row picks a
  // THREAD — that is what the inbox lists now. A parameter-free keyboard or
  // gamepad activation still cycles MESSAGES, the finer grain the response
  // controls act on. Every entry point below keeps the pair consistent:
  // picking a message opens the thread it belongs to, and picking a thread
  // CLEARS the message pin so the thread's active message is recomputed each
  // render — a follow-up in the same thread then becomes the pinned one. No
  // row component owns a competing selection and no host command is sent.
  let selectedMessageId = null;
  let selectedThreadId = null;

  const commsView = (s) => (variant.commsView ? variant.commsView(s) : s);

  function messagesFor(s) {
    const view = s ? commsView(s) : null;
    return Array.isArray(view && view.messages) ? view.messages : [];
  }

  /**
   * The message the console opens on when the operator has picked nothing: a
   * live Critical while it is live, then the first unread, then the newest.
   * Critical remains ordinary, non-modal panel content — it wins the panel's
   * automatic selection, not the screen.
   */
  function autoMessage(messages) {
    return [...messages].reverse().find(
      (message) => isLatestLiveCriticalMessage(message, messages),
    )
      || messages.find((message) => !message.is_read)
      || messages[messages.length - 1]
      || null;
  }

  /**
   * Which thread is open, its whole conversation, and the message inside it
   * whose responses are pinned. ONE resolution, called by the render and by
   * the semantic adapters alike, so a parameter-free Respond can never mean a
   * different message from the one the operator is looking at.
   *
   * Also where a stale pick is dropped: a thread that left the inbox, or a
   * message that did, stops being selected rather than blanking the panel.
   */
  function resolveOpen(messages) {
    if (selectedMessageId != null
        && !messages.some((message) => message && message.id === selectedMessageId)) {
      selectedMessageId = null;
    }
    if (selectedThreadId != null
        && !messages.some((message) => effectiveThreadId(message) === selectedThreadId)) {
      selectedThreadId = null;
    }

    let threadId = selectedThreadId;
    if (threadId == null) {
      const auto = autoMessage(messages);
      threadId = auto ? effectiveThreadId(auto) : null;
    }
    if (threadId == null) return { threadId: null, messages: [], active: null };

    const conversation = threadMessagesFrom(messages, threadId);
    const picked = selectedMessageId == null
      ? null
      : conversation.find((message) => message && message.id === selectedMessageId);
    const active = picked
      || activeThreadMessageFrom(messages, threadId)
      || conversation[conversation.length - 1]
      || null;
    return { threadId, messages: conversation, active };
  }

  /**
   * Show the open thread's panel — and say so to the shell only when saying so
   * is TRUE.
   *
   * The panel is an ordinary column member on a wide screen and a local
   * `.overlay-panel` in phone portrait — ONE node, and the console stylesheet
   * decides which, so nothing here reads a media query. It declares no
   * `data-tab-code`, so the shell's Station Bar never offers it as a tab: it
   * belongs to the row that was tapped, not to the seat. Its own ← Back
   * button (delegated by `initConsoleOverlays`) is the way out.
   *
   * `setConsoleOverlay` is the SHELL-FACING seam, though: `console-core` reads
   * `.overlay-panel.open` back out of the DOM and posts it to the bar as this
   * console's open overlay, and the bar then treats the seat's own tab as
   * "come back to the console" instead of the per-system damage popup (issue
   * #1374). In portrait that is exactly right — the panel really is covering
   * the console, and one tap returns to the inbox. Where the panel is an
   * ordinary column it covers NOTHING, so reporting it would eat the first tap
   * on the Comms tab over a panel the operator can already see beside the list.
   *
   * So the stylesheet stays the single decider and this reads back the DOM fact
   * it produced: a panel taken out of flow is an overlay, a panel still in flow
   * is a column. No media query, no viewport arithmetic, and a hull that
   * restyles the panel moves this with it.
   */
  function openThreadPanel(doc) {
    if (!ids.threadPanel) return;
    const root = doc || (typeof document !== 'undefined' ? document : null);
    if (!root) return;
    const panel = root.getElementById(ids.threadPanel);
    const win = root.defaultView;
    if (!panel || !win || typeof win.getComputedStyle !== 'function') return;
    const position = win.getComputedStyle(panel).position;
    if (position !== 'absolute' && position !== 'fixed') {
      // Not overlaying anything. Clear any open mark left over from a portrait
      // session so the bar is not still told an overlay is up after a rotation.
      setConsoleOverlay(null, root);
      return;
    }
    setConsoleOverlay(ids.threadPanel, root);
  }

  /**
   * @param {object} s   the (shape-normalised) console payload
   * @param {Document} [doc]  the document to render into; defaults to the
   *   ambient `document` in a browser. A vitest suite passes a jsdom document.
   */
  function renderStation(s, doc) {
    doc = doc || (typeof document !== 'undefined' ? document : null);
    if (!doc || !s) return;

    // The comms view the panels read from — `s` itself for the battleship's
    // flat `comms` family, a metadata-selected family slice for the cruiser.
    const view = commsView(s);
    const msgs = Array.isArray(view.messages) ? view.messages : [];
    const contacts = Array.isArray(view.contacts) ? view.contacts : [];
    const threads = sortedThreadsFrom(msgs, contacts);
    const open = resolveOpen(msgs);
    const openSummary = open.threadId == null
      ? null
      : threads.find((thread) => thread.thread_id === open.threadId) || null;

    // ── Contact list (the CONTACTS tab) ──────────────────────────────────
    if (ids.contactList) {
      const el = doc.getElementById(ids.contactList);
      if (el) el.state = { contacts };
    }

    // ── Hail list (the HAILS tab): one row per thread ────────────────────
    if (ids.hailList) {
      const el = doc.getElementById(ids.hailList);
      if (el) el.state = { ...view, threads, selected_thread_id: open.threadId };
    }

    // ── The open thread: the whole conversation, responses pinned ────────
    if (ids.currentMessage) {
      const el = doc.getElementById(ids.currentMessage);
      if (el) {
        el.state = {
          thread: open.active,
          messages: open.messages,
          sender_name: openSummary ? openSummary.sender_name : null,
          rejection: view.rejection,
        };
      }
    }

    // ── Unread count on the HAILS tab ────────────────────────────────────
    // Threads, not messages: the tab counts the conversations still waiting
    // on the operator, which is the number of rows a tap can act on.
    if (ids.hailsUnread) {
      const el = doc.getElementById(ids.hailsUnread);
      if (el) {
        const unread = unreadThreadCount(threads);
        el.textContent = unread > 0 ? String(unread) : '';
        el.hidden = unread === 0;
        el.title = t('console.comms.seg.hails_unread', { n: unread });
      }
    }

    // ── Active-hail readout ──────────────────────────────────────────────
    // The thread summary's name rather than the last speaker's: a
    // multi-speaker thread is one channel, and the readout names the channel.
    if (ids.activeHail) {
      const el = doc.getElementById(ids.activeHail);
      if (el) {
        el.textContent = openSummary
          ? (openSummary.sender_name || t('console.common.active_hail'))
          : t('console.common.no_active_hail');
      }
    }

    // ── AUTO badge ───────────────────────────────────────────────────────
    if (ids.autoBadge) {
      const el = doc.getElementById(ids.autoBadge);
      if (el) setAutoState(null, el, variant.autoState ? !!variant.autoState(s, view) : !!view.comms_auto);
    }

    // ── Bespoke per-hull tail (the cruiser's Navigation view, ...) ───────
    if (variant.tail) variant.tail(s, view, doc, t, open.active);
  }

  /**
   * Apply one real local MESSAGE selection and synchronously repaint it.
   *
   * An explicit id comes from a visible control. A parameter-free binding
   * selects the automatic message first, then cycles in wire order. The
   * message's own thread comes with it, so the list highlight, the open
   * conversation and the pinned responses all move together. The semantic
   * adapter reports Applied only after this function returns true.
   *
   * The cycle continues from wherever the operator actually IS, which is not
   * always a pinned id: tapping a HAILS row picks a thread and deliberately
   * leaves the message unpinned (see `selectThread`), so the anchor is then the
   * message that thread currently resolves to. Only with nothing picked at all
   * does a press select the automatic message rather than advance past it.
   */
  renderStation.selectMessage = function selectMessage(s, messageId, doc) {
    const messages = messagesFor(s);
    if (messages.length === 0) return false;
    let nextId = typeof messageId === 'string' && messageId ? messageId : null;
    if (!nextId) {
      const anchorId = selectedMessageId != null
        ? selectedMessageId
        : (selectedThreadId != null ? (resolveOpen(messages).active?.id ?? null) : null);
      if (anchorId != null) {
        const currentIndex = messages.findIndex(
          (message) => message && message.id === anchorId,
        );
        nextId = messages[(currentIndex + 1 + messages.length) % messages.length]?.id || null;
      } else {
        nextId = resolveOpen(messages).active?.id || null;
      }
    } else if (!messages.some((message) => message && message.id === nextId)) {
      return false;
    }
    if (!nextId) return false;
    const picked = messages.find((message) => message && message.id === nextId) || null;
    selectedMessageId = nextId;
    selectedThreadId = picked ? effectiveThreadId(picked) : null;
    openThreadPanel(doc);
    renderStation(s, doc);
    return true;
  };

  /**
   * Apply one real local THREAD selection and synchronously repaint it.
   *
   * This is what a HAILS row activates — pointer tap and Enter/Space alike.
   * A thread-grain pick stays thread-grain: it clears the message pin rather
   * than freezing the thread on whichever message was active at tap time. That
   * is what keeps a live conversation live — a scripted follow-up arrives as a
   * NEW message carrying the SAME `thread_id` (`CommsMessage::injected`), so
   * pinning the tapped message would leave the operator staring at the
   * already-answered one's disabled responses with no cue that the reply they
   * are owed is unreachable. With the pin cleared, `resolveOpen` recomputes the
   * active message on every render and the pinned responses track the thread's
   * newest actionable message. An explicit MESSAGE pick — the keyboard cycle,
   * or a control naming an exact id — still wins, and still overrides this.
   */
  renderStation.selectThread = function selectThread(s, threadId, doc) {
    if (typeof threadId !== 'string' || !threadId) return false;
    const messages = messagesFor(s);
    if (threadMessagesFrom(messages, threadId).length === 0) return false;
    selectedThreadId = threadId;
    selectedMessageId = null;
    openThreadPanel(doc);
    renderStation(s, doc);
    return true;
  };

  /** The message parameter-free Respond and Show act on: the open thread's. */
  renderStation.currentMessage = function currentMessage(s) {
    return resolveOpen(messagesFor(s)).active;
  };

  // A console document is normally single-use; this explicit seam keeps HMR
  // and isolated renderer fixtures from carrying presentation state between
  // sessions without exposing the selected ids as mutable component state.
  renderStation.resetSelection = function resetSelection() {
    selectedMessageId = null;
    selectedThreadId = null;
  };

  return renderStation;
}
