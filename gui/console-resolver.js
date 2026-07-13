export function resolveConsoleUrl(shipStations, stationId) {
  const stations = (shipStations && shipStations.stations) || [];
  const station = stations.find(s => s.id === stationId);
  return (station && station.console) || null;
}

if (typeof window !== 'undefined') {
  window.resolveConsoleUrl = resolveConsoleUrl;
}
