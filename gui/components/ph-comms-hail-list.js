// strings-boot first: its top-level await delays this module's evaluation —
// and therefore this element's registration and upgrade — until the string
// table is loaded, so the constructor's template t() calls never see an
// empty table. No-op in Node tests (setup-strings.js loads the table there).
import '../strings-boot.js';
import { t } from '../strings.js';
import {
  COMMS_PRIORITY,
  sortedThreadsFrom,
} from '../comms-state.js';
import { PhElement, phDefine } from './ph-element.js';
import { installRovingTabindex, syncRovingTabindex } from '../roving-tabindex.js';
import { COMMS_SELECT_MESSAGE_ACTION_ID } from '../stations/comms-actions.js';

/**
 * The Comms inbox: ONE ROW PER THREAD (issue #1380).
 *
 * A five-message conversation with one station is one row — the station's
 * name, the newest line as its preview, and how many messages sit behind it —
 * rather than five rows repeating the same sender. Activating a row opens that
 * thread; showing the whole conversation is `ph-comms-current-message`'s job.
 *
 * `state.threads` is the projection the shared Comms renderer already computed
 * with `sortedThreadsFrom` (live Critical first, then urgent+unread, unread,
 * read). It is optional: given only `state.messages` the component groups them
 * itself through that same pure function, so a fixture — or any surface that
 * has a message list and no renderer — still shows the same inbox.
 */
export class PhCommsHailList extends PhElement {
  #rowCache = new Map();
  #emptyEl = null;
  #roving = null;
  #selectedId = null;

  template() {
    return `
  <style>
    :host { display: block; font-family: 'JetBrains Mono', monospace; color: var(--ink); }
    :host * { box-sizing: border-box; }
    .list { display: flex; flex-direction: column; gap: 0.25rem; }
    .empty { font-size: var(--text-xs); color: var(--ink-dim); text-align: center; padding: 0.75rem 0; letter-spacing: 0.2em; }
    /* The row is a native <button role="option"> (issue #1178): focusable,
       named by its own text, and activating on Enter/Space through the SAME
       click handler the pointer uses. The reset strips the browser chrome so it
       still reads as a list row. */
    .row { display: flex; align-items: center; gap: 0.4rem; width: 100%; margin: 0; font: inherit; text-align: left; background: none; border: 0; color: var(--ink); font-size: var(--text-sm); padding: 0.35rem 0.4rem; cursor: pointer; border-radius: 2px; transition: background 0.15s ease; min-height: var(--control-hit-min); }
    .row:hover { background: var(--cyan-deep); }
    .row[aria-selected="true"] { background: var(--cyan-deep); }
    .row.critical { background: var(--fire-deep); box-shadow: inset 3px 0 0 var(--fire-bright); }
    .row.critical:hover, .row.critical[aria-selected="true"] { background: var(--fire-dim); }
    .dot { width: 0.45rem; height: 0.45rem; border-radius: 50%; flex-shrink: 0; }
    .dot.unread { background: var(--science); }
    .dot.read { background: transparent; }
    .row.critical .dot { background: var(--fire-bright); box-shadow: 0 0 0.35rem var(--fire); }
    .sender { font-weight: 400; color: var(--ink); min-width: 4rem; }
    .sender.unread { font-weight: 700; }
    .preview { color: var(--ink-dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; flex: 1; min-width: 0; }
    /* How many messages are folded into this row. Hidden for a one-message
       thread, where the count would only repeat what the row already is. */
    .count { flex-shrink: 0; font-size: var(--text-xs); color: var(--ink-faint); border: 1px solid var(--line-faint); border-radius: 2px; padding: 0.05rem 0.25rem; }
    .count[hidden] { display: none; }
    .priority-cue { display: none; align-items: center; gap: 0.2rem; flex-shrink: 0; border: 1px solid var(--fire-bright); color: var(--fire-bright); padding: 0.08rem 0.25rem; font-size: var(--text-xs); font-weight: 700; letter-spacing: 0.08em; }
    .priority-cue.critical { display: inline-flex; }
    .priority-shape { line-height: 1; }
    .timestamp { color: var(--edge); font-size: var(--text-xs); flex-shrink: 0; }
  </style>
  <div class="list" id="list"></div>
`;
  }

  connectedCallback() {
    super.connectedCallback();
    // Role + accessible name + keyboard operation (issue #1178). The hails were
    // clickable <div>s the keyboard could not land on; the list is now a proper
    // listbox — one Tab stop, arrows roving over the option rows — named from
    // the same string its console heading already shows.
    this.setAttribute('role', 'listbox');
    this.setAttribute('aria-orientation', 'vertical');
    this.setAttribute('aria-label', t('component.comms_hails.label'));
    // One Tab stop for the whole list; arrows move between the option rows.
    // Native <button>s keep their own Enter/Space activation — no fork.
    this.#roving ??= installRovingTabindex(this, {
      getItems: () => this.#rovingItems(),
      orientation: 'vertical',
    });
    this.#syncRoving();
  }

  /** The list's rovable option rows, in document order. */
  #rovingItems() {
    return Array.from(this.shadowRoot.querySelectorAll('.row'));
  }

  /** Re-establish the single tab stop after a render adds/removes rows. */
  #syncRoving() {
    syncRovingTabindex(this.#rovingItems());
  }

  render(state) {
    const s = state || {};
    const threads = Array.isArray(s.threads)
      ? s.threads
      : sortedThreadsFrom(s.messages, s.contacts);
    const list = this.shadowRoot.getElementById('list');
    // Selection is projected by the shared Comms renderer. This component
    // reflects it but never mutates it optimistically; pointer and bound input
    // therefore converge on the same semantic adapter and one state owner.
    this.#selectedId = typeof s.selected_thread_id === 'string'
      ? s.selected_thread_id : null;

    const live = new Set(threads.map(thread => thread.thread_id || ''));
    for (const [key, el] of this.#rowCache) {
      if (!live.has(key)) { el.remove(); this.#rowCache.delete(key); }
    }

    if (threads.length === 0) {
      if (!this.#emptyEl) { this.#emptyEl = document.createElement('div'); this.#emptyEl.className = 'empty'; this.#emptyEl.textContent = t('component.comms_hails.empty'); list.appendChild(this.#emptyEl); }
      return;
    }
    if (this.#emptyEl) { this.#emptyEl.remove(); this.#emptyEl = null; }

    // The shell pushes console state ~10x a second, and moving a node in the
    // DOM is a remove+insert that drops focus. So remember who holds focus
    // before the loop, move only rows that are genuinely out of place, and put
    // focus back if a real re-sort carried it away (issue #1178's one-Tab-stop,
    // roving-arrows contract has to survive a repaint).
    const focused = this.shadowRoot.activeElement;

    threads.forEach((thread, index) => {
      const id = thread.thread_id || '';
      const sender = thread.sender_name || '';
      const preview = thread.subject || '';
      const unread = !!thread.any_unread;
      const critical = thread.latest_priority === COMMS_PRIORITY.CRITICAL;
      const count = Number.isFinite(thread.message_count) ? thread.message_count : 1;
      let row = this.#rowCache.get(id);
      if (!row) {
        row = document.createElement('button');
        row.type = 'button';
        row.className = 'row';
        row.setAttribute('role', 'option');
        row.innerHTML = '<span class="dot"></span><span class="sender"></span><span class="preview"></span><span class="count" hidden></span><span class="priority-cue"><span class="priority-shape" aria-hidden="true">◆</span><span class="priority-text"></span></span>';
        // Enter/Space (native to the button) and a pointer tap alike run this
        // one handler, dispatching the SAME local semantic selection action —
        // naming the THREAD the row stands for, not one of its messages.
        row.addEventListener('click', () => {
          if (typeof window.activateSemanticAction === 'function') {
            window.activateSemanticAction(COMMS_SELECT_MESSAGE_ACTION_ID, {
              source: 'control', detail: { thread_id: id },
            });
          }
        });
        this.#rowCache.set(id, row);
      }
      // Row order has to follow the projection — the inbox re-sorts as threads
      // are read and answered — but ONLY when it actually differs from it.
      // `#emptyEl` was removed above, so `list.children` holds rows alone and
      // index-for-index comparison is exact; an unchanged order moves nothing.
      const at = list.children[index];
      if (at !== row) list.insertBefore(row, at || null);
      row.dataset.id = id;
      row.dataset.priority = thread.latest_priority || COMMS_PRIORITY.ROUTINE;
      row.classList.toggle('critical', critical);
      row.setAttribute('aria-selected', String(id === this.#selectedId));
      const dot = row.querySelector('.dot');
      const senderEl = row.querySelector('.sender');
      const countEl = row.querySelector('.count');
      const cue = row.querySelector('.priority-cue');
      dot.className = unread ? 'dot unread' : 'dot read';
      senderEl.className = unread ? 'sender unread' : 'sender';
      senderEl.textContent = sender;
      row.querySelector('.preview').textContent = preview;
      countEl.textContent = count > 1 ? String(count) : '';
      countEl.hidden = count <= 1;
      countEl.title = t('component.comms_hails.thread_count', { n: count });
      cue.className = critical ? 'priority-cue critical' : 'priority-cue';
      cue.querySelector('.priority-text').textContent = critical
        ? t('component.comms.priority.critical') : '';
    });
    // A genuine re-sort DID move the focused row; hand focus back to the same
    // node so the operator keeps their place in the list.
    if (focused && focused.isConnected && this.shadowRoot.activeElement !== focused) {
      focused.focus({ preventScroll: true });
    }
    this.#syncRoving();
  }
}

phDefine('ph-comms-hail-list', PhCommsHailList);
