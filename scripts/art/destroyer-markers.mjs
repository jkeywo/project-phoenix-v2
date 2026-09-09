// The new GLB embeds its rig; preserve old attachments in that identity frame.
import assert from 'node:assert/strict';
export function migrateMarkers(reference) {
  const {offset, rotation, scale} = reference.base;
  assert.equal(rotation[0], 0);
  assert.equal(rotation[2], 0);
  const c = Math.cos(rotation[1]), s = Math.sin(rotation[1]);
  const vector = v => {
    const [x, y, z] = v.map((n, i) => n * scale[i]);
    return [c * x + s * z, y, -s * x + c * z];
  };
  return Object.fromEntries(Object.entries(reference.markers).map(([name, marker]) => {
    const direction = vector(marker.direction);
    const length = Math.hypot(...direction);
    assert.ok(length > 0);
    return [name, {...marker,
      position: vector(marker.position).map((n, i) => n + offset[i]),
      direction: direction.map(n => n / length),
    }];
  }));
}
