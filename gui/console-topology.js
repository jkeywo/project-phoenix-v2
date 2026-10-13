/** Console identity is independent of its current Station and screen implementation. */
export function consoleDefinitions(roster) {
  return (roster?.consoles || []).map(c => ({ ...c,
    name: c.name || roster.stations?.find(s => s.id === c.home_station)?.name || c.id }));
}
export function consoleHome(roster, id) {
  return consoleDefinitions(roster).find(c => c.id === id)?.home_station || null;
}
/** Produce a read view, never a second authoritative store. Feeds are typed. */
export function consoleReadState(state, definition) {
  const allowed = new Set(definition.systems || []);
  for (const feed of definition.feeds || []) {
    if (state.blackboardKinds?.[feed.source] === feed.kind) allowed.add(feed.source);
  }
  const select = source => Object.fromEntries(Object.entries(source || {}).filter(([id]) => allowed.has(id)));
  const view = { ...state, blackboards: select(state.blackboards), blackboardKinds: select(state.blackboardKinds),
    blackboardPresentation: select(state.blackboardPresentation),
    consoleSystems: { [definition.id]: definition.systems || [] } };
  // The recipient may host several screens. Exact hull rows follow this
  // Console's equipment and its permitted Repair publication, not that union.
  const visibleHull = new Set(definition.systems || []);
  for (const [source, board] of Object.entries(view.blackboards)) {
    if (view.blackboardKinds[source] === 'Repair') {
      for (const row of board.system_hull || []) visibleHull.add(row.system_id);
    }
  }
  view.consoleHull = (state.consoleHull || []).filter(row => visibleHull.has(row.system_id));
  const hosting = state.consoleHosts?.[definition.id];
  const rating = hosting?.rating || (definition.home_station ? state.stationRatings?.[definition.home_station] : definition.visiting_rating);
  // Legacy family builders read their automation fallback under family keys.
  // This is a read projection only; the client's Station rating controls and
  // authoritative controlSources retain their original seat/System identities.
  view.stationRatings = { ...state.stationRatings };
  view.stationTutorials = { ...state.stationTutorials,
    [definition.id]: definition.tutorials?.length ? definition.tutorials
      : (definition.home_station ? state.stationTutorials?.[definition.home_station] || [] : []) };
  if (rating) for (const id of definition.systems || []) {
    const family = state.systemConsoleFamilies?.[id];
    if (family) view.stationRatings[family] = rating;
  }
  const kinds = new Set(Object.values(view.blackboardKinds));
  // Legacy messages still carry these facts outside their typed blackboards.
  // Bind that publication to the same observation kind instead of allowing a
  // missing board to fall through to another Console's global client cache.
  const channels = [
    [['Helm', 'Sensors', 'Navigation', 'TacticalRadar', 'Weapons'], ['asteroids', 'shipX', 'shipZ', 'shipYaw']],
    [['Helm'], ['forwardSpeed', 'impulseChargeProgress', 'boostEnabled', 'boostBattery', 'boostActive']],
    [['Sensors'], ['sensorsTarget']],
    [['Weapons'], ['weaponsTarget', 'weaponsTargetName', 'weaponsBanks', 'weaponsTubes', 'weaponsTorpedoCount', 'weaponsPhaserMode', 'blasterBanks']],
    [['TacticalRadar'], ['weaponsBlips', 'blips']],
    [['Sensors', 'TacticalRadar'], ['regions']],
    [['Comms'], ['commsContacts', 'commsMessages', 'commsRejection']],
    [['Shields'], ['shieldFacings', 'shieldFocusedFacing']],
    [['Repair'], ['repairTeams']],
    [['Captain'], ['objectives', 'currentView', 'redAlert']],
  ];
  for (const [sources, fields] of channels) {
    for (const field of fields) {
      if (sources.some(kind => kinds.has(kind)) && field in state) view[field] = state[field];
      else delete view[field];
    }
  }
  // The Station cache can hold another co-hosted Console's latest waypoint.
  // Match family selection: own Systems in authored order, then bound sources.
  const navigationSource = [...(definition.systems || []), ...Object.keys(view.blackboards).sort()]
    .find(id => view.blackboardKinds[id] === 'Navigation' && view.blackboards[id]);
  if (navigationSource) view.navigationWaypoint = view.blackboards[navigationSource].navigation_waypoint ?? null;
  else delete view.navigationWaypoint;
  return view;
}
