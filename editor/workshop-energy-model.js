import { authoredAndGeneratedSystems } from './workshop-generated-systems.js';

export const ENERGY_RESERVE_FIELDS = Object.freeze(['capacity', 'max_intake_per_sec', 'passive_loss_at_full_per_sec',
  'charge_heat_per_unit', 'discharge_heat_per_unit', 'passive_cooling_per_sec', 'coolant_heat_per_unit', 'trip_heat', 'recover_heat']);
export const ENERGY_REACTOR_FIELDS = Object.freeze(['sustainable_output_per_sec', 'max_output_per_sec', 'ramp_per_sec', 'initial_output_per_sec', 'initial_energy', 'initial_heat', 'max_cooling_per_sec', 'default_cooling_per_sec']);
export const ENERGY_CONSUMER_FIELDS = Object.freeze(['initial_energy', 'initial_heat', 'continuous_cost_per_sec', 'discrete_cost']);
export const ENERGY_GROUP_FIELDS = Object.freeze(['max_flow_per_sec', 'default_flow_per_sec', 'max_cooling_per_sec', 'default_cooling_per_sec', 'priority', 'restore_delay_secs']);
export const energyAliases = kind => ['power', 'power_reactor', 'power_battery', 'shields', 'phaser_control', 'command', 'captain', 'red_alert', 'viewscreen'].includes(kind);
const mandatoryKinds = new Set(['helm_thrust', 'helm_steering', 'lateral_thrust', 'vertical_thrust', 'helm_impulse', 'helm_boost', 'helm_radar',
  'passive_receiver', 'tactical_radar', 'sensor_radar', 'sensors', 'navigation', 'comms', 'repair', 'tractor', 'dock', 'umbilical', 'transporter', 'security',
  'illumination', 'jamming', 'signal-protection', 'resource-recovery', 'fabrication', 'logistics', 'torpedo_magazine', 'phaser_bank', 'blaster_bank', 'torpedo_tube', 'shield_arc']);
export const requiresEnergy = system => !energyAliases(system.kind) && (system.power_group || mandatoryKinds.has(system.kind));
export function requireCoolantStore(value, link = { service: 'coolant' }) {
  const endpoint = link.provider || value.resource_services?.[link.service || 'coolant'];
  if (endpoint?.kind !== 'capacity' || !(value.infrastructure?.capacity || []).some(row => row.id === endpoint.id))
    throw new Error('Configure a real coolant capacity and its named service in Resources before creating energy equipment.');
}
const reserve = () => ({ capacity: 100, max_intake_per_sec: 20, passive_loss_at_full_per_sec: 1, charge_heat_per_unit: 0.1,
  discharge_heat_per_unit: 0.1, passive_cooling_per_sec: 0.2, coolant_heat_per_unit: 10, trip_heat: 50, recover_heat: 30 });
export const defaultEnergyConsumer = system => ({ system, initial_energy: 50, initial_heat: 0, continuous_cost_per_sec: 1,
  discrete_cost: 1, reserve: reserve(), coolant: { service: 'coolant' } });
export const defaultEnergyGroup = id => ({ id, max_flow_per_sec: 20, default_flow_per_sec: 10,
  max_cooling_per_sec: 5, default_cooling_per_sec: 1, priority: 0, restore_delay_secs: 0.5 });
export function createEnergyModel(value) {
  const systems = authoredAndGeneratedSystems(value).filter(requiresEnergy);
  const missing = systems.filter(system => !system.power_group || !Object.hasOwn(value.power_groups || {}, system.power_group));
  if (missing.length) throw new Error(`Assign authored power groups to machinery first: ${missing.map(system => system.id).join(', ')}.`);
  return { reactor: { sustainable_output_per_sec: 100, max_output_per_sec: 200, ramp_per_sec: 20, initial_output_per_sec: 100,
    initial_energy: 50, initial_heat: 0, max_cooling_per_sec: 5, default_cooling_per_sec: 1, reserve: reserve(), coolant: { service: 'coolant' } },
  consumer: systems.map(system => defaultEnergyConsumer(system.id)),
  group: Object.keys(value.power_groups || {}).sort().map(defaultEnergyGroup) };
}
