export const entityCache = new Map();

export async function loadEntityConfig(path) {
  if (entityCache.has(path)) return entityCache.get(path);
  try {
    const url = path.startsWith('/') ? path : '/' + path;
    const resp = await fetch(url);
    if (!resp.ok) return null;
    const text = await resp.text();
    const config = window.tomlParse(text);
    entityCache.set(path, config);
    return config;
  } catch {
    return null;
  }
}

export function getEntityConfig(path) {
  return entityCache.get(path) || null;
}

const KNOWN_ENTITIES = [
  { name: 'player_ship', path: 'assets/entities/player_ship.toml', tags: ['player', 'ship'] },
  { name: 'pirate_raider', path: 'assets/entities/pirate_raider.toml', tags: ['enemy', 'ship'] },
  { name: 'ship_harrow_warhawk', path: 'assets/entities/ship_harrow_warhawk.toml', tags: ['enemy', 'ship'] },
  { name: 'ship_harrow_patrol', path: 'assets/entities/ship_harrow_patrol.toml', tags: ['enemy', 'ship'] },
  { name: 'ship_requiem_courier', path: 'assets/entities/ship_requiem_courier.toml', tags: ['enemy', 'ship'] },
  { name: 'asteroid_large', path: 'assets/entities/asteroid_large.toml', tags: ['asteroid'] },
  { name: 'asteroid_small', path: 'assets/entities/asteroid_small.toml', tags: ['asteroid'] },
  { name: 'asteroid_cosmetic', path: 'assets/entities/asteroid_cosmetic.toml', tags: ['asteroid', 'cosmetic'] },
  { name: 'asteroid_field_main', path: 'assets/entities/asteroid_field_main.toml', tags: ['asteroid_field'] },
  { name: 'station_axiom', path: 'assets/entities/station_axiom.toml', tags: ['station'] },
  { name: 'station_outpost', path: 'assets/entities/station_outpost.toml', tags: ['station'] },
  { name: 'station_research_outpost', path: 'assets/entities/station_research_outpost.toml', tags: ['station'] },
  { name: 'region_nebula', path: 'assets/entities/region_nebula.toml', tags: ['region', 'nebula'] },
  { name: 'region_kaleth_nebula', path: 'assets/entities/region_kaleth_nebula.toml', tags: ['region', 'nebula'] },
  { name: 'region_radiation_zone', path: 'assets/entities/region_radiation_zone.toml', tags: ['region'] },
  { name: 'star_sun', path: 'assets/entities/star_sun.toml', tags: ['star'] },
  { name: 'planet_earth', path: 'assets/entities/planet_earth.toml', tags: ['planet'] },
];

export function preloadEntityList() {
  return KNOWN_ENTITIES;
}

export async function preloadEntityCache() {
  const results = [];
  for (const ent of KNOWN_ENTITIES) {
    const config = await loadEntityConfig(ent.path);
    results.push({ ...ent, config });
  }
  return results;
}
