/**
 * gui/gm-widgets-panel.js — the typed world-authored GM widget region (issue
 * #1439, PRD #1419 story 13, presentation contract PRD #1418).
 *
 * A scenario author composes a Game Master's desk from FOUR typed widgets and
 * nothing else: a filtered attention list, a Station workload summary, existing
 * permitted GM action buttons, and an authored note. The vocabulary is closed
 * in `src/world/config.rs` (`GM_WIDGET_TYPES`), refused at world load with the
 * `[[gm_role_preset]] #i '<id>' [[gm_role_preset.widget]] #j` that named it, and
 * re-checked here — so there is no authored world, and no poked payload, in
 * which arbitrary markup, a script, a style or a new GM permission reaches this
 * region. That is the whole point of the typed table: an author composes what
 * already exists, they do not write UI.
 *
 * # What this file is NOT
 *
 * It is not a second attention queue and not a second filter implementation.
 * The attention widget seeds and then READS the one controller every GM
 * surface shares (`gui/gm-attention-filters.js`, issue #1433) and the one
 * projection the queue already parsed; the workload widget reads the rows
 * `gui/gm-workload-panel.js` already parsed. Neither fetches, parses or
 * narrows anything of its own. A widget that disagreed with the panel beside
 * it about what is waiting would be worse than no widget.
 *
 * # Authored default filters
 *
 * An `attention` widget may author a band, a category and a ship. They are
 * DEFAULTS applied when the operator selects that preset — the live switch is
 * them asking for this role's view — and never on a reconnect, where the
 * filters and snoozes that operator actually left behind win. The ship is
 * authored as a world entity NAME (the vocabulary `contacts` uses), because
 * the queue's own ship facet is a runtime entity id no author can know; it is
 * resolved against the live queue when the default is applied, and a ship that
 * is not in the queue simply leaves that facet unnarrowed rather than hiding
 * everything.
 *
 * # Actions are the shipped buttons
 *
 * An `actions` widget does not submit anything. It activates the existing GM
 * action control by its DOM id, so the T2 confirmation category
 * (`gui/gm-confirmation.js`), the admission check and the action-feedback
 * lifecycle are the ones that button already had — there is exactly one
 * implementation of pausing a session and this is not it. A control the desk
 * is not currently offering (missing, hidden or disabled) is mirrored as a
 * disabled widget button with a sentence saying so: a button that quietly does
 * nothing is worse than one that is visibly unavailable.
 *
 * # Notes are text
 *
 * A note's `text` is a String Table id. It is rendered with `textContent`, so
 * the desk has no path from authored content to parsed markup at all.
 */

import { GM_ATTENTION_FILTER_ALL } from './gm-attention-filters.js';

/** How many attention rows one widget card lists before it stops. The card is
 * a filtered view for reading, not a replacement queue — the full list, its
 * hold/Return-to-live state and its verbs live in `#gm-attention-panel`. */
export const GM_WIDGET_ATTENTION_ROWS = 5;

/**
 * Mount the widget region.
 *
 * Everything it draws comes from a callback into a surface that already exists,
 * so this module holds no projection state of its own and nothing here can go
 * stale independently of the panel it mirrors.
 */
export function createGmWidgetsPanel({
  doc = globalThis.document,
  t = (id) => id,
  has = () => false,
  filters = null,
  /** The attention queue's own public state (`createGmAttentionPanel().state`). */
  readAttention = () => ({ occurrences: [] }),
  /** The workload advisory's own rows (`createGmWorkloadPanel().state`). */
  readWorkload = () => [],
  /** Redraw the shared attention queue after a default filter is applied. */
  repaintAttention = () => {},
  /** Activate one existing GM action control. */
  activateAction = null,
} = {}) {
  const root = doc && doc.getElementById('gm-widgets');
  const listEl = doc && doc.getElementById('gm-widgets-list');
  let widgets = [];

  const label = (value) => (typeof value === 'string' && has(value) ? t(value) : (value || ''));

  const element = (tag, className, text) => {
    const node = doc.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };

  /** The shipped control an `actions` widget id names, or `null`. */
  const control = (id) => (doc ? doc.getElementById(id) : null);

  /** Activate it exactly as the operator would. Default rather than required so
   * a test can observe the forwarding without a live host. */
  const activate = activateAction || ((id) => {
    const button = control(id);
    if (!button || button.disabled || button.hidden) return false;
    button.click();
    return true;
  });

  /** The live occurrences, already parsed by the queue. */
  const occurrences = () => {
    const state = readAttention();
    return Array.isArray(state && state.occurrences) ? state.occurrences : [];
  };

  /**
   * Resolve an authored ship NAME to the runtime entity id the shared filter
   * speaks, using whichever live surface knows about that hull.
   *
   * `null` when this session has never seen the ship — the honest answer is to
   * leave the facet unnarrowed, because narrowing to an id that exists nowhere
   * would empty every surface and read as "nothing is happening".
   */
  function shipIdForName(name) {
    for (const row of occurrences()) {
      const ship = row.target && row.target.ship;
      if (ship && (ship.name === name || ship.entity_id === name)) return ship.entity_id;
    }
    for (const row of readWorkload()) {
      if (row.ship && (row.ship.name === name || row.ship.entity_id === name)) {
        return row.ship.entity_id;
      }
    }
    return null;
  }

  /**
   * Apply one preset's authored default filters to the ONE shared controller.
   *
   * Called only when the operator selects a preset. Each facet a widget
   * authors is set; a facet it omits is reset to All, so switching from a
   * narrow role to a broad one does not leave the previous role's narrowing on
   * an operator who asked for something else.
   */
  function applyAuthoredDefaults() {
    if (!filters) return;
    const authored = widgets.find((widget) => widget.type === 'attention');
    if (!authored) return;
    filters.setFilter('band', authored.band || GM_ATTENTION_FILTER_ALL);
    filters.setFilter('category', authored.category || GM_ATTENTION_FILTER_ALL);
    const ship = authored.ship ? shipIdForName(authored.ship) : null;
    filters.setFilter('ship', ship || GM_ATTENTION_FILTER_ALL);
    repaintAttention();
  }

  function renderAttention(widget, card) {
    const visible = occurrences().filter((row) => (filters ? filters.visible(row) : true));
    const summary = element('p', 'gm-widget-summary');
    summary.dataset.widgetSummary = widget.id;
    summary.textContent = t('server.gm.widget.attention.summary', { count: visible.length });
    card.appendChild(summary);
    // What this card is narrowed to, in words. A filtered view that does not
    // say what it is hiding is a view an operator cannot trust, and the
    // sentence survives forced colours where a highlighted select would not.
    const narrowing = [
      widget.band ? t(`server.gm.attention.band.${widget.band}`) : null,
      widget.category && has(`server.gm.attention.category.${widget.category}`)
        ? t(`server.gm.attention.category.${widget.category}`) : null,
      widget.ship ? label(widget.ship) : null,
    ].filter(Boolean);
    if (narrowing.length > 0) {
      card.appendChild(element('p', 'gm-widget-narrowing',
        t('server.gm.widget.attention.narrowing', { narrowing: narrowing.join(' · ') })));
    }
    const list = element('ol', 'gm-widget-attention-rows');
    for (const row of visible.slice(0, GM_WIDGET_ATTENTION_ROWS)) {
      const item = doc.createElement('li');
      item.dataset.occurrenceId = row.id;
      item.dataset.band = row.band;
      item.appendChild(element('span', 'gm-widget-band', t(`server.gm.attention.band.${row.band}`)));
      item.appendChild(element('span', 'gm-widget-reason',
        t(row.reason.id, row.reason.params || {})));
      list.appendChild(item);
    }
    card.appendChild(list);
    if (visible.length === 0) {
      card.appendChild(element('p', 'gm-widget-empty', t('server.gm.widget.attention.empty')));
    }
  }

  function renderWorkload(widget, card) {
    const rows = readWorkload()
      .filter((row) => !widget.ship
        || row.ship.name === widget.ship || row.ship.entity_id === widget.ship);
    const summary = element('p', 'gm-widget-summary');
    summary.dataset.widgetSummary = widget.id;
    summary.textContent = t('server.gm.widget.workload.summary', { count: rows.length });
    card.appendChild(summary);
    const list = element('ul', 'gm-widget-workload-rows');
    for (const row of rows) {
      const item = doc.createElement('li');
      item.dataset.stationKey = row.key;
      item.dataset.level = row.level;
      item.appendChild(element('span', 'gm-widget-station',
        t('server.gm.workload.row', { ship: label(row.ship.name), station: label(row.station_name) })));
      // The level is a WORD, never a colour alone — PRD #1418 story 25.
      item.appendChild(element('span', 'gm-widget-level',
        t(`server.gm.workload.state.${row.level}`)));
      list.appendChild(item);
    }
    card.appendChild(list);
    if (rows.length === 0) {
      card.appendChild(element('p', 'gm-widget-empty', t('server.gm.widget.workload.empty')));
    }
  }

  function renderActions(widget, card) {
    const group = element('div', 'gm-widget-actions');
    group.setAttribute('role', 'group');
    let unavailable = 0;
    for (const id of widget.actions) {
      const shipped = control(id);
      const button = doc.createElement('button');
      button.type = 'button';
      button.className = 'btn';
      button.dataset.widgetAction = id;
      // The widget button is a second PRESS of one control, so it wears that
      // control's own name rather than a name this file invents for it.
      button.textContent = shipped && shipped.textContent ? shipped.textContent : id;
      const offered = Boolean(shipped) && !shipped.disabled && !shipped.hidden;
      button.disabled = !offered;
      if (!offered) unavailable += 1;
      button.addEventListener('click', () => activate(id));
      group.appendChild(button);
    }
    card.appendChild(group);
    if (unavailable > 0) {
      card.appendChild(element('p', 'gm-widget-unavailable',
        t('server.gm.widget.actions.unavailable', { count: unavailable })));
    }
  }

  function renderNote(widget, card) {
    const note = element('p', 'gm-widget-note');
    note.dataset.noteId = widget.text;
    // `textContent`, always. An authored note is text the String Table owns;
    // there is no branch here in which it becomes markup.
    note.textContent = t(widget.text);
    card.appendChild(note);
  }

  const RENDERERS = {
    attention: renderAttention,
    workload: renderWorkload,
    actions: renderActions,
    note: renderNote,
  };

  function render() {
    if (!listEl) return;
    const active = doc.activeElement;
    const focusedAction = active && active.dataset && active.dataset.widgetAction
      ? active.dataset.widgetAction : null;
    listEl.replaceChildren(...widgets.map((widget) => {
      const card = doc.createElement('li');
      card.dataset.widgetId = widget.id;
      card.dataset.widgetType = widget.type;
      const heading = element('h3', 'gm-widget-heading', label(widget.label));
      heading.id = `gm-widget-${widget.id}-heading`;
      card.setAttribute('role', 'group');
      card.setAttribute('aria-labelledby', heading.id);
      card.appendChild(heading);
      RENDERERS[widget.type](widget, card);
      return card;
    }));
    if (root) root.hidden = widgets.length === 0;
    // A repaint must not drop the keyboard. The only focusable thing this
    // region draws is an action button, and it comes back under the same id.
    if (focusedAction) {
      const again = listEl.querySelector(`button[data-widget-action="${focusedAction}"]`);
      if (again && !again.disabled) again.focus();
    }
  }

  /**
   * The effective role preset changed.
   *
   * `source` comes straight from `gui/gm-role-presets.js`: `'select'` is the
   * operator's own live switch and is the only one that applies the authored
   * default filters. A preset the world no longer declares resolves to the
   * built-in All, whose widget list is empty, so the region simply goes away —
   * the removed-preset fallback needs no separate branch.
   */
  function setPreset(preset, { source = 'available' } = {}) {
    widgets = Array.isArray(preset && preset.widgets) ? [...preset.widgets] : [];
    if (source === 'select') applyAuthoredDefaults();
    render();
  }

  render();

  return {
    setPreset,
    /** Redraw from surfaces that already hold the state. Called when the queue,
     * the workload advisory or the operator's own filters changed. */
    repaint: render,
    reset() { widgets = []; render(); },
    dispose() { widgets = []; if (listEl) listEl.replaceChildren(); if (root) root.hidden = true; },
    state: () => ({
      widgets: widgets.map((widget) => ({ ...widget })),
      rendered: listEl
        ? [...listEl.querySelectorAll('li[data-widget-id]')].map((card) => card.dataset.widgetId)
        : [],
    }),
  };
}
