/** Presentation only: both hosts compose their existing presenters into one desk.
 *
 * The composition is the design canvas's "After M5 - facilitation + operations"
 * artboard (PRD #930): one bar, three columns, two rows. The artboard's nine
 * grid cells first became six REGIONS - a region is the artboard's panel frame
 * and the sections inside it are the blocks that frame holds - and issues #1502
 * and #1503 then migrated most of those panels into the Live dock workspace,
 * leaving TWO grid children: the dock across the left and centre, and the
 * detail column. Nothing changed column on the way: each panel became a dock
 * panel in the place its region already held. */
import { phAdoptConsoleStyles } from './components/ph-console-styles.js';
import { healthStateLabelId } from './gm-health-banner.js';
import { formatAttentionAge } from './gm-attention-panel.js';
import { liveLayoutModel } from './live-layout-model.js';
import { mountDockLayout } from './workshop-layout-renderer.js';

/** Workload levels that are a claim about a PERSON, worst last. A hull's word
 * is the worst of these across its Stations; a hull whose Stations are all
 * Backfill (or all Offline) says that instead, and a hull the advisory has not
 * published rows for says nothing at all. */
export const GM_ROSTER_WORKLOAD_RANK = Object.freeze(['underused', 'engaged', 'overloaded']);

/** The desk panels that are registered dock panels rather than grid regions,
 * as `[dock panel id, DOM id, availability DOM id?]`. The third entry names the
 * node whose `hidden` says whether the panel has anything to show, for a panel
 * whose registered node is a host its content mounts into later — without it
 * the dock would read an empty wrapper nothing ever hides. The three log views kept their tab
 * relationship: the dock's own default arrangement puts them in one group,
 * with the dock's roving tablist semantics, and docking may pull them apart. */
export const GM_LIVE_DOCK_PANEL_IDS = Object.freeze([
  ['mission', 'gm-mission-panel'],
  ['comms', 'gm-comms-panel'],
  ['activity', 'gm-activity'],
  ['journal', 'gm-journal'],
  ['session-history', 'gm-session-history'],
  ['map', 'gm-map-panel'],
  ['attention', 'gm-attention-panel'],
  ['workload', 'gm-workload-panel'],
  ['widgets', 'gm-widgets'],
  ['health', 'gm-health-panel'],
  ['station', 'gm-station-tools'],
  ['station-console', 'gm-station-surface'],
  ['presentation', 'gm-presentation-dock'],
  ['audition', 'gm-audition-dock'],
  // The handoff shows itself only when a retained source pack exists, and that
  // is the panel's own decision — the dock reads it and offers no empty tab.
  ['source-link', 'gm-source-link-dock', 'gm-workshop-source'],
]);

export function mountGmWorkspaceShell({ doc, win, t, has, selectEntity, native = false }) {
  const root = doc.getElementById('gm-console');
  if (!root) return { refresh() {}, metadata() {}, selection() {}, dispose() {},
    mountLiveLayout() {}, setLiveLayout() {}, liveLayoutState() { return null; },
    showLog() { return false; } };
  phAdoptConsoleStyles(doc);
  const css = doc.createElement('link');
  css.rel = 'stylesheet';
  css.href = new URL('./gm-workspace.css', import.meta.url).href;
  doc.head.append(css);
  const liveWorkspace = native || win.__phoenixGmPage === true
    || doc.documentElement.classList.contains('phoenix-gm-page');
  const dockCss = liveWorkspace ? doc.createElement('link') : null;
  if (dockCss) {
    dockCss.rel = 'stylesheet';
    dockCss.href = new URL('./dock-layout.css', import.meta.url).href;
    doc.head.append(dockCss);
  }
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
  const ships = element('div', 'gm-roster-ships');
  const operators = element('div', 'gm-roster-operators');
  roster.append(ships, element('h3', null, 'server.gm.shell.operators'), operators);
  move(roster, 'gm-spawn-panel');
  const region = (id, stacked) => {
    const el = element('div', id);
    el.className = stacked ? 'gm-desk-region gm-desk-stack' : 'gm-desk-region';
    return el;
  };
  // The dock workspace is the desk's left and centre: since issue #1503 the
  // map, the attention queue, Station workload, the authored widget region and
  // peer health are registered panels in it rather than grid regions.
  //
  // The map IS reparented now, which it never was before. There is no way to
  // move a node between parents without disconnecting it, so
  // <ph-navigation-map> restores its render loop and size observer in
  // `connectedCallback` instead of only in `onTemplate`. Every other migrated
  // panel moves node-and-listeners intact, exactly as the roster panels do.
  // The dock workspace is NOT a panel frame: it carries no padding, border or
  // chrome and opens no scroll box of its own, because each dock panel scrolls
  // inside its own frame. Giving it `.gm-desk-region` would put a second
  // scroller in the chain — the nested-scroll fault the region layout exists to
  // avoid — and clip floating panels at the region's chamfered corners.
  const brief = element('div', 'gm-desk-brief');
  brief.className = 'gm-desk-dock';
  const detail = region('gm-desk-detail', true);
  const mapPanel = get('gm-map-panel');
  desk.insertBefore(brief, mapPanel);
  mapPanel.after(detail);
  const liveSurface = element('div', 'gm-live-layout');
  liveSurface.className = 'gm-live-layout';
  brief.append(liveSurface);
  // Right: the selected hull, then the checkpoints a live event is recovered
  // from (issues #1445/#1446) at the bottom of the column, as the artboard
  // draws them. The restore control travels inside #gm-checkpoint.
  move(detail, 'gm-inspector', 'gm-checkpoint');
  get('gm-comms-text')?.parentElement.classList.add('gm-comms-draft');
  // The session history is its own record now: it was a disclosure inside the
  // activity feed, and a dock panel carries its own header.
  const sessionHistory = element('section', 'gm-session-history');
  move(sessionHistory, 'gm-session-log-heading', 'gm-session-log');
  // It has to be IN the document from here on: gui/gm-session-controls.js finds
  // #gm-session-log by id, and on the ordinary browser host there is no dock to
  // place it. The dock moves it out into its own frame when it mounts.
  get('gm-activity')?.append(sessionHistory);
  const inspector = get('gm-inspector');
  // The saved action history (issue #1441) joins the inspector column: it is
  // the desk's detail/reading column, it already stacks and scrolls its own
  // sections, and that is what keeps the journal legible at 200% text rather
  // than competing for one of the fixed grid cells.
  move(inspector, 'gm-system-panel', 'gm-contact-panel', 'gm-npc-panel', 'gm-objective-panel', 'gm-despawn-panel', 'gm-faction-panel');
  // Authentic Station operation is two dock panels (issue #1504): the pending
  // state and the takeover controls are an ordinary tool, and the console
  // itself is a document. Neither is a new command route — the puppet still
  // owns the iframe, its typed bridge and the capability gate — and neither is
  // another simulation participant.
  const stationTools = element('section', 'gm-station-tools');
  const stationSurface = element('section', 'gm-station-surface');
  stationSurface.hidden = true;
  // Both wrappers join the document BEFORE anything moves into them. The
  // console iframe starts inside #gm-station-controls, so moving the controls
  // into a detached wrapper would take the iframe out of the document with them
  // and `getElementById` would stop finding it for the move that follows.
  root.append(stationTools, stationSurface);
  move(stationTools, 'gm-station-pending', 'gm-station-controls');
  move(stationSurface, 'gm-station-frame', 'gm-station-activity-heading', 'gm-station-activity');
  // The operator's own utilities mount LATER than this shell and resolve their
  // host by id, so each gets a stable one now. Each host sits where its panel
  // used to, which is what the ordinary browser host — which has no dock — still
  // sees; the dock moves them into their frames when it mounts.
  const presentationHost = element('div', 'gm-presentation-dock');
  const auditionHost = element('div', 'gm-audition-dock');
  get('gm-mission-panel')?.append(presentationHost, auditionHost);
  const sourceLinkHost = element('div', 'gm-source-link-dock');
  brief.append(sourceLinkHost);
  // Built here, once every wrapper this shell composes exists: the dock is
  // handed NODES rather than ids, which also survives a re-mount when the
  // migrated panels already live in the previous canvas.
  const liveDockNodes = new Map(GM_LIVE_DOCK_PANEL_IDS.map(([panel, id]) => [panel,
    { 'gm-session-history': sessionHistory, 'gm-station-tools': stationTools,
      'gm-station-surface': stationSurface, 'gm-presentation-dock': presentationHost,
      'gm-audition-dock': auditionHost, 'gm-source-link-dock': sourceLinkHost }[id] || get(id)]));
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
  // The chips are laid over the map's own grid rather than inserted into its
  // canvas, so they travel with it when it is docked.
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
  const observer = new win.MutationObserver(() => { styleButtons(); liveLayout?.syncAvailability(); });
  // `hidden` is watched as well as the tree: the role preset toggles Comms and
  // Activity without any other signal reaching this shell, and a tab for a
  // panel the preset has put away is a control that leads nowhere.
  observer.observe(root, { childList: true, subtree: true, attributes: true, attributeFilter: ['hidden'] });
  styleButtons();
  get('gm-station-toggle')?.addEventListener('click', () => {
    // Taking a Station over brings its console to hand. In the dock that is a
    // reveal; without one it is still the scroll it always was. A console the
    // operator CLOSED stays closed — closing it is a decision, and this is a
    // convenience — and a console with nothing to show has nothing to bring.
    if (stationSurface.hidden) return;
    if (liveLayout?.reveal('station-console', { reopen: false })) return;
    if (liveLayout) return;
    stationSurface.scrollIntoView?.({ block: 'start' });
  });
  let liveLayout = null;
  const dockOrigins = new Map();
  function preserveForDock(node, attributes = []) {
    if (!node) return;
    if (!dockOrigins.has(node)) {
      const marker = doc.createComment(`gm-live-dock:${node.id}`);
      node.before(marker);
      dockOrigins.set(node, { marker, attributes: new Map() });
    }
    const origin = dockOrigins.get(node);
    for (const name of attributes) {
      if (!origin.attributes.has(name)) origin.attributes.set(name, node.getAttribute(name));
    }
  }
  function restoreDockedNodes() {
    for (const [node, origin] of dockOrigins) {
      origin.marker.replaceWith(node);
      for (const [name, value] of origin.attributes) {
        if (value === null) node.removeAttribute(name); else node.setAttribute(name, value);
      }
    }
    dockOrigins.clear();
  }
  function mountLiveLayout(initial, onChange) {
    if (!liveWorkspace) return null;
    liveLayout?.dispose();
    const readiness = element('section', 'gm-readiness-dock');
    const startControls = get('gm-start-controls');
    preserveForDock(startControls);
    if (startControls) readiness.append(startControls);
    const join = get('gm-join-controls');
    preserveForDock(join, ['style']);
    join?.removeAttribute('style');
    const manual = element('section', 'gm-manual-save-dock');
    const manualPanel = get('manual-save-panel');
    preserveForDock(manualPanel, native ? ['hidden'] : []);
    if (manualPanel) manual.append(manualPanel);
    if (native && typeof win.__hostGmCheckpointCreate !== 'function') {
      manualPanel?.setAttribute('hidden', '');
      manual.append(element('p', 'gm-native-manual-save-unavailable', 'server.gm.shell.manual_save_unavailable'));
    }
    const migrated = {};
    for (const [panel] of GM_LIVE_DOCK_PANEL_IDS) {
      const node = liveDockNodes.get(panel);
      // The role preset owns `hidden` on these panels (gui/gm-role-presets.js).
      // It stays the only writer: the dock READS that attribute through
      // `available` below, so a preset and the arrangement can never race.
      preserveForDock(node);
      migrated[panel] = node || element('section');
    }
    liveLayout = mountDockLayout({ root, surface: liveSurface,
      panels: { roster, readiness, join: join || element('section'), 'manual-save': manual, ...migrated },
      available: panel => {
        const source = GM_LIVE_DOCK_PANEL_IDS.find(([name]) => name === panel)?.[2];
        // A panel that declares an availability node and has not mounted it yet
        // has nothing to show, so it is not offered. The tree observer brings it
        // back the moment that node arrives.
        if (source) return !!get(source) && !get(source).hidden;
        return !liveDockNodes.get(panel)?.hidden;
      },
      onVisible(visible) {
        // A map with no frame draws 60 times a second into a canvas of no size.
        liveDockNodes.get('map')?.querySelector('ph-navigation-map')
          ?.setRendering?.(visible.has('map'));
      },
      labels: {
        switcher: t('server.gm.shell.layout.switcher'), reset: t('server.gm.shell.layout.reset'),
        float: t('server.gm.shell.layout.float'), close: t('server.gm.shell.layout.close'),
        dock: Object.fromEntries(['left', 'right', 'top', 'bottom', 'tab'].map(place =>
          [place, t(`server.gm.shell.layout.dock_${place}`)])),
        panels: Object.fromEntries(liveLayoutModel.panels.map(panel =>
          [panel, t(`server.gm.shell.layout.panel.${panel.replace(/-/g, '_')}`)])),
        tabs: t('server.gm.shell.log_tabs'),
      },
      // The migrated panels' contents are owned by modules that resolve them by
      // id AFTER this mounts, so no registered panel may leave the document.
      retain: true,
      initial, onChange, model: liveLayoutModel, viewportNarrow: true, doc, win });
    styleButtons();
    return liveLayout;
  }
  if (liveWorkspace) mountLiveLayout(liveLayoutModel.defaultLayout());
  return {
    dispose() {
      liveLayout?.dispose();
      restoreDockedNodes();
      observer.disconnect();
      dockCss?.remove();
      css.remove();
    },
    mountLiveLayout,
    setLiveLayout(value) { liveLayout?.set(value); },
    liveLayoutState() { return liveLayout?.state() || null; },
    /** Bring the attention region to the front for a banner that has just been
     * drawn into it. A technical banner is rendered verbatim so a Game Master
     * cannot hide a connection or recovery failure from themselves; a panel
     * sitting behind another tab would hide it just as effectively as a role
     * preset would, so the arrangement yields to it. */
    revealBanners() { return liveLayout?.reveal('attention') === true; },
    /** Bring one of the migrated record panels to the front.
     *
     * A panel that is not the active tab is `hidden`, so anything inside it is
     * unfocusable and unclickable — which would silently break the navigations
     * the desk already has: the attention queue opening an authored Comms
     * route, or any later surface pointing at the action log. Callers ask for
     * the PANEL they are about to touch and the dock reveals it; a panel the
     * effective role preset has put away is left alone, because a preset
     * hiding a panel is a decision, not an accident. */
    showLog(panelId) {
      const entry = GM_LIVE_DOCK_PANEL_IDS.find(([, id]) => id === panelId);
      const panel = entry && liveDockNodes.get(entry[0]);
      if (!entry || !panel || panel.hidden) return false;
      return liveLayout?.reveal(entry[0]) === true;
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
      // The role preset may have just put a record panel away or brought it
      // back; the dock re-reads that rather than being told twice.
      liveLayout?.syncAvailability();
      paintRoster();
    },
  };
}
