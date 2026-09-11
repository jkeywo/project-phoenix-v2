/** Presentation only: both hosts compose their existing presenters into one desk.
 *
 * The composition is the design canvas's "After M5 - facilitation + operations"
 * artboard (PRD #930): one bar, three columns, two rows, six REGIONS. A region
 * is the artboard's panel frame; the sections inside it are the blocks that
 * frame holds. Nothing changed column against the After-M3 screen this file
 * already drew - each milestone adds panels to the column that already held
 * its kind. */
import { phAdoptConsoleStyles } from './components/ph-console-styles.js';
import { healthStateLabelId } from './gm-health-banner.js';
import { formatAttentionAge } from './gm-attention-panel.js';

/** Workload levels that are a claim about a PERSON, worst last. A hull's word
 * is the worst of these across its Stations; a hull whose Stations are all
 * Backfill (or all Offline) says that instead, and a hull the advisory has not
 * published rows for says nothing at all. */
export const GM_ROSTER_WORKLOAD_RANK = Object.freeze(['underused', 'engaged', 'overloaded']);

/** The three views that share the desk's centre-bottom region, in tab order. */
export const GM_DESK_LOG_VIEWS = Object.freeze([
  ['comms', 'gm-comms-panel'],
  ['activity', 'gm-activity'],
  ['journal', 'gm-journal'],
]);

export function mountGmWorkspaceShell({ doc, win, t, has, selectEntity }) {
  const root = doc.getElementById('gm-console');
  if (!root) return { refresh() {}, metadata() {}, selection() {}, dispose() {} };
  phAdoptConsoleStyles(doc);
  const css = doc.createElement('link');
  css.rel = 'stylesheet';
  css.href = new URL('./gm-workspace.css', import.meta.url).href;
  doc.head.append(css);
  root.classList.add('gm-desk');
  const get = id => doc.getElementById(id);
  const move = (parent, ...ids) => ids.forEach(id => { if (get(id)) parent.append(get(id)); });
  function element(tag, id, key) {
    const el = doc.createElement(tag);
    if (id) el.id = id;
    if (key) { el.dataset.i18n = key; el.textContent = t(key); }
    return el;
  }
  const bar = root.querySelector('header');
  bar.classList.add('gm-station-bar');
  bar.querySelector('p')?.remove();
  const identity = element('span', 'gm-operator-identity');
  const scenario = element('span', 'gm-scenario-title');
  scenario.hidden = true;
  bar.insertBefore(scenario, get('gm-role-preset-label'));
  const clock = element('span', 'gm-session-clock');
  clock.hidden = true;
  const peers = element('span', 'gm-peer-summary');
  bar.insertBefore(clock, get('gm-role-preset-label'));
  bar.insertBefore(peers, get('gm-role-preset-label'));
  // The artboard's bar pills (Tick / Quiet / Checkpoint). Each one is drawn
  // ONLY from a payload that has actually landed - gm_health for the tick,
  // gm_attention's quiet-time occurrence, this browser's own checkpoint
  // catalogue - so a pill that is missing is a fact this desk does not have
  // rather than one it invented. There is deliberately no "60 Hz" and no
  // "2 min ago": neither a rate nor a wall-clock age is on any projection.
  const healthPills = element('div', 'gm-health-pills');
  healthPills.setAttribute('role', 'group');
  healthPills.setAttribute('aria-label', t('server.gm.shell.pills'));
  bar.insertBefore(healthPills, get('gm-role-preset-label'));
  const lethal = element('label', 'gm-lethal-label', 'server.gm.shell.lethal');
  const modes = element('select', 'gm-lethal-mode');
  for (const mode of ['immediate', 'confirm', 'confirm-preview']) {
    const option = element('option', null, `server.gm.shell.mode.${mode}`);
    option.value = mode;
    modes.append(option);
  }
  lethal.append(modes);
  bar.append(lethal);
  move(bar, 'gm-session-controls');
  bar.append(identity);
  modes.addEventListener('change', () => {
    win.__hostGmConfirmationProfile?.setMode('effect.lethal', modes.value);
    modes.value = win.__hostGmConfirmationProfile?.mode('effect.lethal') || 'confirm-preview';
  });
  const desk = get('gm-workspace');
  const roster = element('section', 'gm-roster');
  roster.setAttribute('aria-labelledby', 'gm-roster-heading');
  roster.append(element('h2', 'gm-roster-heading', 'server.gm.shell.roster'));
  if (win.__phoenixGmPage || doc.documentElement.classList.contains('phoenix-gm-page')) {
    // Readiness must remain above a growing roster and the spawn palette.
    // The ordinary viewscreen keeps these controls in its own lobby.
    move(roster, 'gm-start-controls');
    move(roster, 'gm-join-controls');
    roster.querySelector('#gm-join-controls')?.removeAttribute('style');
  }
  const ships = element('div', 'gm-roster-ships');
  const operators = element('div', 'gm-roster-operators');
  roster.append(ships, element('h3', null, 'server.gm.shell.operators'), operators);
  move(roster, 'gm-spawn-panel');
  // The viewscreen still needs its lobby controls. Move them only on a GM page.
  if (win.__phoenixGmPage || doc.documentElement.classList.contains('phoenix-gm-page')) {
    move(roster, 'manual-save-panel');
  }
  const region = (id, stacked) => {
    const el = element('div', id);
    el.className = stacked ? 'gm-desk-region gm-desk-stack' : 'gm-desk-region';
    return el;
  };
  const brief = region('gm-desk-brief', true);
  const detail = region('gm-desk-detail', true);
  const logRegion = region('gm-desk-log', false);
  // The map is NEVER reparented: moving it would disconnect
  // <ph-navigation-map> and cancel its render loop. The regions are placed
  // around the cell it already occupies, and the panels move into regions that
  // are already in the document so `getElementById` keeps finding them.
  const mapPanel = get('gm-map-panel');
  for (const id of ['gm-map-panel', 'gm-mission-panel', 'gm-health-panel']) {
    get(id)?.classList.add('gm-desk-region');
  }
  desk.insertBefore(brief, mapPanel);
  mapPanel.after(detail);
  detail.after(get('gm-mission-panel'), logRegion, get('gm-health-panel'));
  // Left: what is waiting, who is flying it, and whatever the world authored.
  // `gm-widgets` is last on purpose — gui/gm-widgets-panel.js owns its
  // `hidden` state, which is why it is absent from GM_ROLE_PRESET_PANEL_IDS:
  // a second writer there would race it, exactly as it would for the Station
  // puppet's controls.
  move(brief, 'gm-attention-panel');
  brief.append(roster);
  move(brief, 'gm-workload-panel', 'gm-widgets');
  // Right: the selected hull, then the checkpoints a live event is recovered
  // from (issues #1445/#1446) at the bottom of the column, as the artboard
  // draws them. The restore control travels inside #gm-checkpoint.
  move(detail, 'gm-inspector', 'gm-checkpoint');
  // Centre-bottom: Comms, the activity feed and the saved action journal share
  // ONE region behind a tab strip.
  move(logRegion, 'gm-comms-panel', 'gm-activity', 'gm-journal');
  get('gm-comms-text')?.parentElement.classList.add('gm-comms-draft');
  const sessionHistory = element('details', 'gm-session-history');
  sessionHistory.append(element('summary', null, 'server.gm.session.log_heading'));
  move(sessionHistory, 'gm-session-log-heading', 'gm-session-log');
  get('gm-activity').append(sessionHistory);
  const inspector = get('gm-inspector');
  // The saved action history (issue #1441) joins the inspector column: it is
  // the desk's detail/reading column, it already stacks and scrolls its own
  // sections, and that is what keeps the journal legible at 200% text rather
  // than competing for one of the fixed grid cells.
  move(inspector, 'gm-system-panel', 'gm-contact-panel', 'gm-npc-panel', 'gm-objective-panel', 'gm-station-pending', 'gm-station-controls', 'gm-despawn-panel', 'gm-faction-panel');
  const stationSurface = element('section', 'gm-station-surface');
  stationSurface.hidden = true;
  move(stationSurface, 'gm-station-frame', 'gm-station-activity-heading', 'gm-station-activity');
  root.append(stationSurface);
  // The centre-bottom tab strip. It switches views with `data-log-view` on the
  // region and NEVER with `hidden` on the panels: `hidden` on Comms and
  // Activity belongs to the role preset (gui/gm-role-presets.js,
  // GM_ROLE_PRESET_PANEL_IDS), and two writers on one attribute is the race
  // the authored widget region is deliberately kept out of. A preset that
  // hides a panel hides its TAB, which this shell owns.
  const logTabs = element('div', 'gm-desk-log-tabs');
  logTabs.setAttribute('role', 'tablist');
  logTabs.setAttribute('aria-label', t('server.gm.shell.log_tabs'));
  logRegion.prepend(logTabs);
  let logView = GM_DESK_LOG_VIEWS[0][0];
  function setLogView(view) {
    logView = view;
    logRegion.dataset.logView = view;
    for (const [name] of GM_DESK_LOG_VIEWS) {
      const button = get(`gm-log-tab-${name}`);
      if (!button) continue;
      button.setAttribute('aria-selected', String(name === view));
      button.tabIndex = name === view ? 0 : -1;
    }
  }
  for (const [view, panelId] of GM_DESK_LOG_VIEWS) {
    const panel = get(panelId);
    const button = element('button', `gm-log-tab-${view}`, `server.gm.shell.log.${view}`);
    button.type = 'button';
    button.setAttribute('role', 'tab');
    button.dataset.logView = view;
    if (panel) {
      button.setAttribute('aria-controls', panelId);
      panel.setAttribute('role', 'tabpanel');
      panel.setAttribute('aria-labelledby', button.id);
    }
    button.addEventListener('keydown', event => {
      const buttons = [...logTabs.children].filter(node => !node.hidden);
      const index = buttons.indexOf(button);
      if (index < 0) return;
      const next = event.key === 'ArrowRight' ? (index + 1) % buttons.length
        : event.key === 'ArrowLeft' ? (index + buttons.length - 1) % buttons.length
        : event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : null;
      if (next !== null) { event.preventDefault(); buttons[next].click(); buttons[next].focus(); }
    });
    button.addEventListener('click', () => setLogView(view));
    logTabs.append(button);
  }
  setLogView(logView);
  /** Keep the strip honest about what the effective role preset is showing. A
   * hidden panel has no tab, and an operator standing on a tab that just went
   * away lands on the first view that is still there. */
  function reconcileLogTabs() {
    let fallback = null;
    let visible = false;
    for (const [view, panelId] of GM_DESK_LOG_VIEWS) {
      const panel = get(panelId);
      const button = get(`gm-log-tab-${view}`);
      if (!button) continue;
      const available = !!panel && !panel.hidden;
      if (button.hidden !== !available) button.hidden = !available;
      if (available && fallback === null) fallback = view;
      if (available && view === logView) visible = true;
    }
    if (!visible && fallback !== null) setLogView(fallback);
  }
  const tabs = element('div', 'gm-inspector-tabs');
  tabs.setAttribute('role', 'tablist');
  const knowledge = get('gm-knowledge-panel');
  const comparison = element('div', 'gm-comparison');
  move(comparison, 'gm-knowledge-pending', 'gm-knowledge-panel');
  inspector.insertBefore(tabs, inspector.children[1]);
  inspector.insertBefore(comparison, tabs.nextSibling);
  // Crew knowledge chooses an observing ship independently of the action
  // target (for example, revealing an NPC to that player's Sensors).
  const knowledgeScope = element('div', 'gm-knowledge-scope');
  knowledgeScope.hidden = true;
  const knowledgeLabel = knowledge?.querySelector('label[for="gm-knowledge-select"]');
  if (knowledgeLabel) knowledgeScope.append(knowledgeLabel);
  move(knowledgeScope, 'gm-knowledge-select');
  inspector.insertBefore(knowledgeScope, comparison);
  for (const mode of ['truth', 'crew', 'difference']) {
    const button = element('button', `gm-tab-${mode}`, `server.gm.shell.tab.${mode}`);
    button.type = 'button'; button.setAttribute('role', 'tab');
    button.addEventListener('keydown', event => {
      const buttons = [...tabs.children];
      const index = buttons.indexOf(button);
      const next = event.key === 'ArrowRight' ? (index + 1) % buttons.length
        : event.key === 'ArrowLeft' ? (index + buttons.length - 1) % buttons.length
        : event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : null;
      if (next !== null) { event.preventDefault(); buttons[next].click(); buttons[next].focus(); }
    });
    button.setAttribute('aria-controls', mode === 'truth' ? 'gm-entity-card' : 'gm-comparison');
    button.addEventListener('click', () => setTab(mode));
    tabs.append(button);
  }
  function setTab(mode) {
    inspector.dataset.view = mode;
    comparison.hidden = mode === 'truth';
    tabs.querySelectorAll('button').forEach(button => {
      button.setAttribute('aria-selected', String(button.id === `gm-tab-${mode}`));
      button.tabIndex = button.id === `gm-tab-${mode}` ? 0 : -1;
    });
  }
  setTab('truth');
  const chip = element('p', 'gm-selection-chip');
  get('gm-map-panel').insertBefore(chip, get('gm-map-legend'));
  const armedChip = element('p', 'gm-armed-chip');
  get('gm-map-panel').insertBefore(armedChip, get('gm-map-legend'));
  // Keep the upgraded map connected: moving it would cancel its render loop.
  // CSS shares its grid cell with the chip overlay instead of reparenting it.
  const mapChips = element('div', 'gm-map-chips');
  mapChips.append(chip, armedChip);
  get('gm-map-panel').insertBefore(mapChips, get('gm-map-legend'));
  const systems = element('div', 'gm-system-pills');
  get('gm-entity-card').append(systems);
  const shortcuts = element('div', 'gm-action-grid');
  get('gm-entity-card').after(shortcuts);
  for (const [target, key] of [
    ['gm-effect-amount', 'server.gm.effect.damage'], ['gm-effect-amount', 'server.gm.effect.heal'],
    ['gm-system-select', 'server.gm.system.disable'], ['gm-system-select', 'server.gm.system.restore'],
    ['gm-contact-observer', 'server.gm.contact.reveal'], ['gm-contact-observer', 'server.gm.contact.conceal'],
    ['gm-npc-choice', 'server.gm.npc.heading'], ['gm-objective-panel', 'server.gm.objective.heading'],
  ]) {
    const button = element('button', null, key); button.type = 'button';
    button.setAttribute('aria-controls', target);
    button.addEventListener('click', () => {
      const control = get(target);
      control?.scrollIntoView?.({ block: 'nearest' });
      (control?.matches('input,select,button') ? control : control?.querySelector('button'))?.focus({ preventScroll: true });
      back.hidden = false;
    });
    shortcuts.append(button);
  }
  // The way back up from a quick action's control: a sticky button at the
  // top of the inspector, shown only after a jump and cleared once the
  // selection card is in view again (whether by this button or by scrolling).
  const back = element('button', 'gm-inspector-back', 'server.gm.shell.back_to_selection');
  back.type = 'button';
  back.hidden = true;
  inspector.prepend(back);
  // The scroll box is the REGION, not the inspector: the post-M5 right-hand
  // column holds the inspector and the checkpoints in one frame, and only the
  // frame scrolls (gui/gm-workspace.css explains why a second nested scroller
  // there is a correctness problem, not a layout preference).
  const detailScroller = inspector.closest('.gm-desk-region') || inspector;
  back.addEventListener('click', () => {
    back.hidden = true;
    if (typeof detailScroller.scrollTo === 'function') detailScroller.scrollTo({ top: 0 }); else detailScroller.scrollTop = 0;
    get('gm-entity-card')?.scrollIntoView?.({ block: 'start' });
    tabs.querySelector('button[aria-selected="true"]')?.focus({ preventScroll: true });
  });
  detailScroller.addEventListener('scroll', () => { if (!back.hidden && detailScroller.scrollTop < 8) back.hidden = true; });
  let gms = [];
  let entities = [];
  let selected = null;
  let pillSignature = null;
  let stationProjection = { ships: [] };
  let rosterSignature = '';
  const segments = [get('gm-role-preset-select'), modes].filter(Boolean).map(select => {
    const group = element('span'); group.className = 'gm-segment';
    select.after(group); select.classList.add('gm-segment-source');
    select.tabIndex = -1;
    select.setAttribute('aria-hidden', 'true');
    group.setAttribute('role', 'group');
    group.setAttribute('aria-label', t(select === modes ? 'server.gm.shell.lethal' : 'server.gm.role_preset.heading'));
    let signature = '';
    function paint() {
      const next = JSON.stringify([...select.options].map(option => [option.value, option.textContent]));
      if (next !== signature) {
        signature = next; group.replaceChildren();
        for (const option of select.options) {
          const button = element('button'); button.type = 'button'; button.textContent = option.textContent;
          button.dataset.value = option.value;
          button.addEventListener('click', () => {
            select.value = option.value; select.dispatchEvent(new win.Event('change')); paint();
          });
          group.append(button);
        }
      }
      group.querySelectorAll('button').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.value === select.value)));
    }
    select.addEventListener('change', paint);
    return paint;
  });
  const label = value => has(value) ? t(value) : value;
  /** One workload WORD for a hull, from the rows issue #1438 actually
   * published for its Stations. Never invented: a hull with no rows gets no
   * pill, which is what a scenario with the advisory disabled looks like. */
  function workloadWord(entityId) {
    const rows = (win.__hostGmWorkloadState?.() || [])
      .filter(row => row?.ship?.entity_id === entityId);
    if (!rows.length) return null;
    const counted = rows.filter(row => GM_ROSTER_WORKLOAD_RANK.includes(row.level));
    if (counted.length) {
      return counted.reduce((worst, row) => (
        GM_ROSTER_WORKLOAD_RANK.indexOf(row.level) > GM_ROSTER_WORKLOAD_RANK.indexOf(worst)
          ? row.level : worst), GM_ROSTER_WORKLOAD_RANK[0]);
    }
    return rows.every(row => row.level === rows[0].level) ? rows[0].level : null;
  }
  /** The bar's health pills. Signature-guarded like the roster: an unchanged
   * bar is never rebuilt, so nothing under an operator's pointer moves. */
  function paintHealthPills() {
    const nodes = [];
    const health = win.__hostGmHealthState?.();
    const projection = health?.projection;
    // A gm_health payload that has actually landed says SOMETHING: a peer, a
    // Station, an operator, an alert or a deliberate pause. An untouched
    // projection is the panel's own empty seed and claims nothing.
    if (projection && (projection.peers.length || projection.stations.length
      || projection.operators.length || projection.alerts.length || projection.paused)) {
      const pill = element('span');
      pill.dataset.pill = 'tick';
      pill.dataset.state = health.worst;
      pill.textContent = t('server.gm.shell.pill.tick', { state: t(healthStateLabelId(health.worst)) });
      nodes.push(pill);
    }
    const quiet = (win.__hostGmAttentionState?.().occurrences || [])
      .find(row => row.category === 'quiet_time');
    if (quiet) {
      const pill = element('span');
      pill.dataset.pill = 'quiet';
      pill.textContent = t('server.gm.shell.pill.quiet', { age: formatAttentionAge(quiet.age_ms) });
      nodes.push(pill);
    }
    // The LAST checkpoint is the one with the highest capture tick, not
    // whichever row the catalogue happened to hand back first.
    const checkpoints = win.__hostGmCheckpointState?.().rows || [];
    const latest = checkpoints.reduce((best, row) => {
      const tick = Number.parseInt(row.captureTick, 10);
      const bestTick = best ? Number.parseInt(best.captureTick, 10) : Number.NaN;
      if (!Number.isFinite(tick)) return best;
      return !Number.isFinite(bestTick) || tick > bestTick ? row : best;
    }, null) || checkpoints[0] || null;
    if (latest) {
      const pill = element('span');
      pill.dataset.pill = 'checkpoint';
      pill.textContent = t('server.gm.shell.pill.checkpoint', {
        name: latest.kind === 'autosave' ? t('server.gm.checkpoint.autosave') : latest.displayName,
        tick: latest.captureTick || t('server.gm.checkpoint.unknown_tick'),
      });
      nodes.push(pill);
    }
    const signature = nodes.map(node => `${node.dataset.pill}:${node.dataset.state || ''}:${node.textContent}`).join('|');
    if (signature === pillSignature) return;
    pillSignature = signature;
    healthPills.replaceChildren(...nodes);
  }
  function paintRoster() {
    // Only the DERIVED word goes in, never the advisory itself: a gm_workload
    // row carries `sustained_secs`, which moves about once a simulated second
    // while any Station holds demand. Folding the raw projection in here would
    // rebuild every roster button a second — discarding a mousedown held on a
    // row, dropping hover and active state, and losing a screen reader's place
    // in the ship list — for a pill whose word had not changed.
    const signature = JSON.stringify([entities.map(({ entity_id, name, kind, faction }) => ({ entity_id, name, kind, faction })), stationProjection.ships.map(ship => [ship.ship_id, ship.stations, ship.ship_config?.station_systems, ship.control_sources]), gms, entities.map(({ entity_id }) => workloadWord(entity_id))]);
    if (signature === rosterSignature) return;
    rosterSignature = signature;
    const focused = doc.activeElement?.dataset?.entityId;
    ships.replaceChildren();
    for (const kind of ['player_ship', 'npc_ship']) {
      ships.append(element('h3', null, `server.gm.shell.${kind}`));
      for (const entity of entities.filter(row => row.kind === kind)) {
        const row = element('div'); row.className = 'gm-roster-row';
        const button = element('button'); button.type = 'button';
        button.dataset.entityId = entity.entity_id;
        button.setAttribute('aria-pressed', String(entity.entity_id === selected));
        button.textContent = label(entity.name) || entity.entity_id;
        button.addEventListener('click', () => selectEntity(entity.entity_id));
        row.append(button);
        if (entity.faction) {
          const faction = element('span'); faction.textContent = label(entity.faction.name); row.append(faction);
        }
        const stationShip = stationProjection.ships.find(ship => ship.ship_id === entity.entity_id);
        const pills = element('div'); pills.className = 'gm-station-pills';
        for (const station of stationShip?.stations || []) {
          const pill = element('span');
          const owned = stationShip.ship_config?.station_systems?.[station.station_id] || [];
          const sources = new Set(owned.map(id => stationShip.control_sources?.[id]).filter(Boolean));
          const source = sources.size > 1 ? 'mixed' : [...sources][0];
          const stateKey = { Human: 'server.gm.shell.control.human', Ai: 'server.gm.shell.control.ai',
            Offline: 'server.gm.shell.control.offline', mixed: 'server.gm.shell.control.mixed' }[source];
          pill.textContent = `${label(station.name)} · ${stateKey ? t(stateKey) : station.rating}`;
          pill.title = station.rating;
          pill.dataset.rating = station.rating;
          pills.append(pill);
        }
        row.append(pills);
        // The artboard's workload word beside a crewed hull (issue #1438 data,
        // not a second advisory): a WORD, with the panel below still carrying
        // the evidence behind it.
        const level = workloadWord(entity.entity_id);
        if (level) {
          const word = element('span');
          word.className = 'gm-roster-workload';
          word.dataset.level = level;
          word.textContent = t(`server.gm.workload.state.${level}`);
          row.append(word);
        }
        ships.append(row);
      }
    }
    if (focused) [...ships.querySelectorAll('button')].find(button => button.dataset.entityId === focused)?.focus({ preventScroll: true });
    operators.replaceChildren();
    for (const gm of gms) {
      const row = element('p');
      row.textContent = `${gm.name || gm.id} · ${t(gm.connected === false ? 'server.gm.shell.disconnected' : gm.ready ? 'server.gm.shell.ready' : 'server.gm.shell.waiting')}`;
      operators.append(row);
    }
    peers.textContent = t('server.gm.shell.peers', { gms: gms.length, ships: entities.filter(row => row.kind === 'player_ship').length });
  }
  // Presenters repaint button text. Restore the shared family's background
  // and label after each paint, without changing listeners or control ids.
  function styleButtons() {
    segments.forEach(paint => paint());
    root.querySelectorAll('button').forEach(button => {
      button.classList.add('btn', 'btn--md');
      if (button.querySelector('.btn-bg')) return;
      const label = element('span'); label.className = 'label';
      label.append(...button.childNodes);
      const background = element('span'); background.className = 'btn-bg';
      background.setAttribute('aria-hidden', 'true');
      button.append(background, label);
    });
  }
  const observer = new win.MutationObserver(() => { styleButtons(); reconcileLogTabs(); });
  // `hidden` is watched as well as the tree: the role preset toggles Comms and
  // Activity without any other signal reaching this shell, and a tab for a
  // panel the preset has put away is a control that leads nowhere.
  observer.observe(root, { childList: true, subtree: true, attributes: true, attributeFilter: ['hidden'] });
  styleButtons();
  reconcileLogTabs();
  get('gm-station-toggle')?.addEventListener('click', () => {
    if (!get('gm-station-frame').hidden) stationSurface.scrollIntoView?.({ block: 'start' });
  });
  return {
    dispose() { observer.disconnect(); css.remove(); },
    /** Bring one of the centre region's three views to the front.
     *
     * A view that is not current is `display: none`, so anything inside it is
     * unfocusable and unclickable — which would silently break the navigations
     * the desk already has: the attention queue opening an authored Comms
     * route, or any later surface pointing at the action log. Callers ask for
     * the PANEL they are about to touch and this resolves the view; a panel the
     * effective role preset has put away is left alone, because a preset
     * hiding a panel is a decision, not an accident. */
    showLog(panelId) {
      const entry = GM_DESK_LOG_VIEWS.find(([, id]) => id === panelId);
      const panel = entry && get(panelId);
      if (!entry || !panel || panel.hidden) return false;
      setLogView(entry[0]);
      return true;
    },
    metadata(value) { gms = value.gms || []; paintRoster(); },
    selection(entity) {
      selected = entity?.entity_id || null;
      chip.textContent = entity ? label(entity.name) : t('server.gm.inspector.empty');
      systems.replaceChildren();
      for (const system of entity?.status.systems || []) {
        const pill = element('span');
        pill.textContent = `${label(system.name)} · ${system.max_milli_hp ? Math.round(system.current_milli_hp / system.max_milli_hp * 100) : 0}%`;
        systems.append(pill);
      }
      ships.querySelectorAll('button').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.entityId === selected)));
    },
    refresh(projection, stations) {
      if (projection) entities = projection.entities;
      if (stations) stationProjection = stations;
      knowledgeScope.hidden = !knowledge || knowledge.hidden;
      stationSurface.hidden = !get('gm-station-controls') || get('gm-station-controls').hidden;
      const gm = win.__hostLocalGm?.();
      identity.textContent = gm ? `${gm.name || gm.id}` : '';
      const title = get('lobby-title')?.textContent;
      scenario.hidden = !title || title === t('server.loading');
      scenario.textContent = scenario.hidden ? '' : title;
      modes.value = win.__hostGmConfirmationProfile?.mode('effect.lethal') || 'confirm-preview';
      if (typeof win.wasm_sim_tick === 'function') {
        clock.hidden = false;
        clock.textContent = t('server.gm.shell.tick', { tick: win.wasm_sim_tick() });
      }
      if (get('gm-spawn-panel')) get('gm-spawn-panel').dataset.empty = String(!win.__hostGmSpawnState?.().palette);
      if (get('gm-comms-panel')) get('gm-comms-panel').dataset.empty = String(!win.__hostGmCommsState?.().routes.length);
      const armed = win.__hostGmSpawnState?.().arming;
      armedChip.hidden = !armed;
      armedChip.textContent = armed ? t('server.gm.shell.armed', { palette: armed }) : '';
      paintHealthPills();
      reconcileLogTabs();
      paintRoster();
    },
  };
}
