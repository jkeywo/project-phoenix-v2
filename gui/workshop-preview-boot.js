import { installWorkshopTestChild } from '../editor/workshop-test-child.js';
import { launchWorkshopPreview } from '../editor/workshop-preview-runtime.js';

/** The preview's own control vocabulary — local renderer commands only.
 *
 * Deliberately exhaustive and shape-exact, like the Test child's: this page
 * holds a whole captured draft, and the only thing its parent may ask of it is
 * where to point the camera and how to light what it already has. There is no
 * command here that loads anything, and none that reaches a simulation. */
const PREVIEW_CONTROLS = (value, exact) => {
  const number = candidate => typeof candidate === 'number' && Number.isFinite(candidate);
  if (exact(value, ['command', 'mode', 'level'])) {
    return value.command === 'lod' && ['auto', 'base', 'fixed'].includes(value.mode)
      && (value.level === null || (Number.isSafeInteger(value.level) && value.level >= 0));
  }
  if (exact(value, ['command', 'mode'])) {
    return value.command === 'lighting' && ['off', 'ambient', 'directional'].includes(value.mode);
  }
  if (exact(value, ['command', 'enabled'])) {
    return value.command === 'gizmos' && typeof value.enabled === 'boolean';
  }
  if (exact(value, ['command', 'distance'])) {
    return value.command === 'distance' && number(value.distance) && value.distance > 0;
  }
  if (exact(value, ['command', 'focus', 'radius', 'yaw', 'pitch'])) {
    return value.command === 'camera' && Array.isArray(value.focus) && value.focus.length === 3
      && value.focus.every(number) && number(value.radius) && number(value.yaw) && number(value.pitch);
  }
  return false;
};

installWorkshopTestChild({
  type: 'phoenix-workshop-preview-connect',
  controls: PREVIEW_CONTROLS,
  launch: (snapshot, { signal }) => launchWorkshopPreview(snapshot, { signal }),
});
