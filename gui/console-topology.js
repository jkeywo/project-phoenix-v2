/** Console identity is independent of its current Station and screen implementation. */
export function consoleDefinitions(roster) {
  if (roster?.consoles?.length) return roster.consoles.map(c => ({ ...c,
    name: c.name || roster.stations?.find(s => s.id === c.home_station)?.name || c.id }));
  // Migration adapter; remove after every host publishes explicit Consoles.
  return (roster?.stations || []).map(st => ({ ...st, home_station: st.id, screen: st.console }));
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
  const hosting = state.consoleHosts?.[definition.id];
  const rating = hosting?.rating || state.stationRatings?.[definition.home_station];
  // Legacy family builders read their automation fallback under family keys.
  // This is a read projection only; the client's Station rating controls and
  // authoritative controlSources retain their original seat/System identities.
  view.stationRatings = { ...state.stationRatings };
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
    [['Navigation'], ['navigationWaypoint']],
    [['Sensors'], ['sensorsTarget']],
    [['Weapons'], ['weaponsTarget', 'weaponsTargetName', 'weaponsBanks', 'weaponsTubes', 'weaponsTorpedoCount', 'weaponsPhaserMode', 'blasterBanks']],
    [['TacticalRadar'], ['weaponsBlips', 'blips']],
    [['Sensors', 'TacticalRadar'], ['regions']],
    [['Comms'], ['commsContacts', 'commsMessages', 'commsRejection']],
    [['Shields'], ['shieldFacings', 'shieldFocusedFacing']],
    [['Repair'], ['repairTeams']],
    [['Captain'], ['objectives']],
  ];
  for (const [sources, fields] of channels) {
    if (!sources.some(kind => kinds.has(kind))) for (const field of fields) delete view[field];
  }
  return view;
}
