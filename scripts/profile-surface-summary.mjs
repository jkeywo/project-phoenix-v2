// Presentation callbacks and native paints are different rates. Count uploads
// by visible native UI identity, including a zero for a visible stalled view.
export function liveSurfaceRates(events, seconds) {
  if (!(seconds > 0) || !Number.isFinite(seconds)) throw new Error('Invalid sample duration');
  const isUi = surface => surface?.visible && ['gm', 'console'].includes(surface.kind);
  const ids = [...new Set(events.filter(e => isUi(e.surface)).map(e => e.surface.id))];
  return Object.fromEntries(ids.map(id => [id,
    events.filter(e => e.event === 'uploaded' && isUi(e.surface) && e.surface.id === id).length / seconds,
  ]));
}
