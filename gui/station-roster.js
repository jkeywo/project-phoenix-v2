/**
 * gui/station-roster.js — Pure fold from (players, ship station defs) to the
 * lobby station roster (issue #827).
 *
 * Extracted from client.html's rebuildStations(): given the connected player
 * list and the ship's station definitions, produce the per-station roster rows
 * (holder name/token resolved) plus the derived lobby aggregates. The caller
 * (client.html) writes the result onto uiState and schedules a render.
 */

/**
 * @param {Array<{ token: string, name?: string, connected?: boolean,
 *                 ready?: boolean, station?: string|{id: string}|null }>} players
 * @param {Array<{ id: string, name?: string, short_code?: string,
 *                 rank?: string, ratings?: string[] }>} stationDefs
 *        `uiState.shipStations.stations` from Welcome.
 * @returns {{ stations: Array<{ id, name, short_code, rank, holder_name,
 *                               holder_token, ratings }>,
 *             maxPlayers: number, allFilled: boolean, allReady: boolean }}
 */
export function buildStationRoster(players, stationDefs) {
  const defs = stationDefs || [];
  const list = players || [];
  const stations = defs.map(def => {
    // Post issue #619 a player holds a lowercase station id directly.
    const holder = list.find(p => {
      if (!p.connected) return false;
      const held = typeof p.station === 'string' ? p.station
        : (p.station && typeof p.station.id === 'string' ? p.station.id : null);
      return held === def.id;
    });
    return {
      id: def.id,
      name: def.name || '',
      short_code: def.short_code || '',
      rank: def.rank || '',
      holder_name: holder ? holder.name : null,
      holder_token: holder ? holder.token : null,
      ratings: def.ratings || [],
    };
  });
  const allFilled = stations.length > 0 && stations.every(st => st.holder_name);
  const allReady = list.length > 0 && list.every(p => p.ready)
    || list.length === 0;
  return { stations, maxPlayers: defs.length, allFilled, allReady };
}

// Expose for the non-module inline script in client.html.
if (typeof window !== 'undefined') {
  window.buildStationRoster = buildStationRoster;
}
