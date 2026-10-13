import { consoleDefinitions } from './console-topology.js';
export function resolveConsoleUrl(shipStations, stationId) {
  const stations = consoleDefinitions(shipStations);
  const station = stations.find(s => s.id === stationId);
  return (station && station.screen) || null;
}

if (typeof window !== 'undefined') {
  window.resolveConsoleUrl = resolveConsoleUrl;
}
