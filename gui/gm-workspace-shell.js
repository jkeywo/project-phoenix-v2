/** Presentation only: both hosts compose their existing presenters into one desk. */
import { phAdoptConsoleStyles } from './components/ph-console-styles.js';

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
  desk.prepend(roster);
  move(desk, 'gm-attention-panel', 'gm-mission-panel', 'gm-comms-panel', 'gm-activity');
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
  move(inspector, 'gm-system-panel', 'gm-contact-panel', 'gm-npc-panel', 'gm-objective-panel', 'gm-station-pending', 'gm-station-controls', 'gm-despawn-panel', 'gm-faction-panel', 'gm-journal', 'gm-checkpoint');
  const stationSurface = element('section', 'gm-station-surface');
  stationSurface.hidden = true;
  move(stationSurface, 'gm-station-frame', 'gm-station-activity-heading', 'gm-station-activity');
  root.append(stationSurface);
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
  back.addEventListener('click', () => {
    back.hidden = true;
    if (typeof inspector.scrollTo === 'function') inspector.scrollTo({ top: 0 }); else inspector.scrollTop = 0;
    get('gm-entity-card')?.scrollIntoView?.({ block: 'start' });
    tabs.querySelector('button[aria-selected="true"]')?.focus({ preventScroll: true });
  });
  inspector.addEventListener('scroll', () => { if (!back.hidden && inspector.scrollTop < 8) back.hidden = true; });
  let gms = [];
  let entities = [];
  let selected = null;
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
  function paintRoster() {
    const signature = JSON.stringify([entities.map(({ entity_id, name, kind, faction }) => ({ entity_id, name, kind, faction })), stationProjection.ships.map(ship => [ship.ship_id, ship.stations, ship.ship_config?.station_systems, ship.control_sources]), gms]);
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
        row.append(pills); ships.append(row);
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
  const observer = new win.MutationObserver(styleButtons);
  observer.observe(root, { childList: true, subtree: true });
  styleButtons();
  get('gm-station-toggle')?.addEventListener('click', () => {
    if (!get('gm-station-frame').hidden) stationSurface.scrollIntoView?.({ block: 'start' });
  });
  return {
    dispose() { observer.disconnect(); css.remove(); },
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
      paintRoster();
    },
  };
}
