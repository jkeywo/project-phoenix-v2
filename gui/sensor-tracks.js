import { t, wireText } from './strings.js';
import { clampRadarEdge } from './radar-edge.js';

const pair = value => Array.isArray(value) && value.length === 2 && value.every(Number.isFinite);
const stages = new Set(['return', 'identified', 'classified', 'assessed']);
const protection = (value, tick) => value && Number.isInteger(value.incoming_sector) && value.incoming_sector >= 0 && value.incoming_sector < 360 && typeof value.blocked === 'boolean'
  ? { incoming_sector: value.incoming_sector, blocked: value.blocked, sample_tick: Number.isSafeInteger(tick) && tick >= 0 ? tick : null } : null;
const elevation = (value, tick) => value && Number.isFinite(value.height)
  && Number.isFinite(value.error) && value.error >= 0
  && (value.velocity == null || Number.isFinite(value.velocity))
  ? { height: value.height, error: value.error, velocity: value.velocity ?? null,
      sample_tick: Number.isSafeInteger(tick) && tick >= 0 ? tick : null } : null;
export function observedElevationText(value) {
  const held = elevation(value, value?.sample_tick);
  if (!held) return null;
  return t('sensing.track.elevation.value', { height: held.height, error: held.error })
    + (held.sample_tick == null ? '' : ' \u00b7 ' + t('sensing.track.elevation.sample', { tick: held.sample_tick }));
}

export function observedShieldCoverText(value) {
  const held = protection(value, value?.sample_tick);
  if (!held) return null;
  return t(held.blocked ? 'sensing.track.protection.blocked' : 'sensing.track.protection.open', { bearing: held.incoming_sector })
    + (held.sample_tick == null ? '' : ' · ' + t('sensing.track.protection.sample', { tick: held.sample_tick }));
}

/** Project only delivered measurements. Entity snapshots are deliberately not an input. */
export function projectSensorTracks(tracks, { x = 0, z = 0, yaw = 0, range = 0, selected = null, tick = null, edgeClamp = false } = {}) {
  const validRange = Number.isFinite(range) && range > 0;
  const cos = Math.cos(yaw), sin = Math.sin(yaw);
  const rows = [], blips = [], records = new Map();
  for (const track of Array.isArray(tracks) ? tracks : []) {
    if (!track || typeof track.contact_id !== 'string' || !track.contact_id || records.has(track.contact_id)
      || !pair(track.position) || !Number.isFinite(track.position_error) || track.position_error < 0
      || !Number.isSafeInteger(track.observed_tick) || track.observed_tick < 0 || !stages.has(track.stage)) continue;
    records.set(track.contact_id, track);
    const dx = track.position[0] - x, dz = track.position[1] - z;
    const name = track.stage !== 'return' && track.identity?.name ? wireText(track.identity.name) : t('console.sensors.basic_contact');
    const age = Number.isSafeInteger(tick) && tick >= track.observed_tick ? tick - track.observed_tick : null;
    const reported = track.observation?.provenance?.kind === 'sensors_report';
    const visible = validRange && (reported || edgeClamp || Math.hypot(dx, dz) <= range);
    const samples = Array.isArray(track.samples) ? track.samples.filter(sample => sample?.origin && typeof sample.sample_id === 'string') : [];
    const independent = new Set(samples.map(sample => JSON.stringify([sample.origin.source,sample.origin.system])));
    const row = { id: track.contact_id, name, stage: track.stage, predicted: track.predicted === true,
      received: samples.some(sample => Number.isSafeInteger(sample.received_tick)), source_count: independent.size,
      source_details: samples.map(sample => t('sensing.sharing.origin',{source:wireText(sample.origin.source),instrument:wireText(sample.origin.system),tick:sample.observed_tick})),
      elevation: elevation(track.elevation, track.observed_tick),
      error: track.position_error, age, selectable: track.selectable === true && track.predicted === false && visible };
    rows.push(row);
    if (visible) blips.push({
      uuid: row.id, ...clampRadarEdge((dx * cos + dz * sin) / range, (dx * sin - dz * cos) / range, reported || edgeClamp),
      scaled_radius: 0, uncertainty_radius: row.error / range, track: true, predicted: row.predicted,
      icon: null, kind: 'contact', name, label: name, selectable: row.selectable,
    });
  }
  const row = rows.find(row => row.id === selected);
  const track = row && records.get(row.id);
  const dx = track ? track.position[0] - x : null, dz = track ? track.position[1] - z : null;
  const velocity = track && pair(track.velocity) ? track.velocity : null;
  return { blips, tracks: rows, track_mode: true, target_uuid: row?.id ?? null,
    target_track: row ? { ...row, fact_ages: ['identity', 'classification', 'assessment'].flatMap(kind => {
      const observed = track[`${kind}_tick`];
      return track[kind] && Number.isSafeInteger(observed) ? [{ kind, ticks: Number.isSafeInteger(tick) && tick >= observed ? tick - observed : null }] : [];
    }) } : null, target_name: row?.name ?? null,
    target_bearing: track ? (Math.atan2(dx, -dz) * 180 / Math.PI + 360) % 360 : null,
    target_range: track ? Math.hypot(dx, dz) : null,
    target_faction: track?.identity?.faction ? wireText(track.identity.faction) : null,
    target_class: track?.classification ? wireText(track.classification) : null,
    target_speed: velocity ? Math.hypot(...velocity) : null,
    target_heading: velocity && Math.hypot(...velocity) > 0 ? (Math.atan2(velocity[0], -velocity[1]) * 180 / Math.PI + 360) % 360 : null,
    target_protection: protection(track?.assessment?.protection, track?.assessment_tick),
    target_elevation: row?.elevation ?? null,
    target_alert: track?.assessment?.alert ?? null,
    target_weapons: null, target_restrained: track?.assessment?.weapons_restrained ?? null,
    target_disabled: track?.assessment?.disabled ?? null,
    target_kind: null, target_stance: null, target_hull_pct: null, target_threat: null,
    target_shield_freq: null, target_shields: null, target_shield_fraction: null, target_report: null,
    target_projection: null, target_references: Array.isArray(track?.references) ? track.references : [],
  };
}

export function sensorTrackText(row) {
  return [row.name, t(`sensing.track.stage.${row.stage}`),
    t(row.predicted ? 'sensing.track.predicted' : 'sensing.track.measured'),
    t('sensing.track.error', { radius: Math.round(row.error * 10) / 10 }),
    row.age === null ? t('observation.unknown') : t('observation.ticks', { ticks: row.age }),
    ...(row.received ? [t('sensing.sharing.received',{count:row.source_count})] : [])].join(' · ');
}
