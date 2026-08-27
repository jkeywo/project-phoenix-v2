/** Attach explicit host-projection metadata to legacy renderer fixtures. */
const FIXTURE_FAMILIES = Object.freeze({
  captain: 'captain', viewscreen: 'captain', 'red-alert': 'captain',
  sensors: 'sensors', 'sensor-radar': 'sensors',
  'shields-system': 'shields', 'shield-arc-fore': 'shields', 'shield-arc-aft': 'shields',
  'power-reactor': 'power', 'power-battery': 'power', repair: 'repair',
  navigation: 'navigation', comms: 'comms',
  'tactical-radar': 'tactical', 'phaser-control': 'tactical', 'phaser-omni': 'tactical',
  'blaster-fore': 'tactical', 'blaster-port': 'tactical', 'blaster-starboard': 'tactical',
  'helm-thrust': 'helm', 'helm-joystick': 'helm', 'helm-steering': 'helm',
  tractor: 'tractor', umbilical: 'umbilical',
});

export function withConsoleFamilyProjection(payload, overrides = {}) {
  if (!payload?.systems) return payload;
  const systemIds = Object.keys(payload.systems);
  const systemFamilies = { ...(payload.system_families || {}) };
  for (const id of systemIds) {
    const family = overrides[id] || FIXTURE_FAMILIES[id];
    if (family) systemFamilies[id] = family;
  }
  return {
    ...payload,
    system_ids: payload.system_ids || systemIds,
    system_families: systemFamilies,
  };
}
