/** Shared reported/target marker edge treatment; measured world positions stay intact. */
export function clampRadarEdge(radar_x, radar_y, enabled) {
  const distance = Math.hypot(radar_x, radar_y);
  const edge = enabled && distance > 1;
  const scale = edge ? 0.96 / distance : 1;
  return { radar_x: radar_x * scale, radar_y: radar_y * scale, edge };
}
