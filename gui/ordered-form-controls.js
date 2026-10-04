export { liveFormPositions, formNeighbours, survivingFormPosition } from '../editor/ordered-form.js';

/** Rebuild and enable controls before selecting the adapter's focus fallback. */
export function bindFormMoves(up, down, move, rebuild, focus) {
  for (const [control, direction] of [[up, -1], [down, 1]]) {
    control.addEventListener('click', () => {
      const target = move(direction);
      if (target == null) return;
      rebuild();
      focus(target, direction < 0 ? 'up' : 'down', direction < 0 ? 'down' : 'up');
    });
  }
}
