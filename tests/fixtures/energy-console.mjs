/** An explicit small hardware projection for isolated browser control journeys. */
export function energyConsolePayload() {
  const group = (id, priority, parent = null) => ({ id, label: id.toUpperCase(), parent, priority,
    desired_flow_per_sec: 4, effective_flow_per_sec: id === 'weapons' ? 0 : 4, max_flow_per_sec: 12,
    desired_cooling_per_sec: 1, effective_cooling_per_sec: 1, max_cooling_per_sec: 4, cut: id === 'weapons' });
  return { console_id: 'power', system_id: 'main-reactor', power_auto: false, battery_charge: 72, battery_max: 100, battery_online: true, charging: true,
    draining: false, consoles: [], total: 0, total_max: 0,
    energy: { mode: 'manual', cooling_usage: 'low',
      reactor: { energy: 72, capacity: 100, heat: 5, trip_heat: 40, recover_heat: 20, thermal_stopped: false,
        output_target_per_sec: 12, output_actual_per_sec: 11, sustainable_output_per_sec: 12, max_output_per_sec: 20, cooling_per_sec: 1, max_cooling_per_sec: 4 },
      groups: [group('helm', 0), group('weapons', 1), group('shields', 2), group('engine', 0, 'helm')],
      consumers: [{ system: 'drive', group: 'engine', energy: 30, capacity: 40, heat: 3, trip_heat: 20, recover_heat: 10,
        thermal_stopped: false, intake_per_sec: 1, max_intake_per_sec: 3, flow_override_per_sec: null }],
      coolant: [{ provider: 'coolant', available: 20, credit: 0.25 }] } };
}
