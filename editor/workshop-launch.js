const PANELS = new Set(['files', 'source', 'models', 'model-preview', 'composition', 'scripts']);
const SAFE_PATH = /^(?![A-Za-z]:)(?!\/)(?!.*(?:^|\/)\.\.(?:\/|$))[A-Za-z0-9_.\-/]+$/;
const MODEL = /^assets\/models\/[A-Za-z0-9_.\-/]+\.glb$/;
const ENTITY = /^assets\/entities\/[A-Za-z0-9_.\-/]+\.toml$/;
const VARIANT = /^[A-Za-z0-9_-]{1,64}$/;

const cleanPath = value => typeof value === 'string' && SAFE_PATH.test(value) ? value : null;

/** One bounded, presentation-only entry contract for bookmarks and launchers. */
export function parseWorkshopLaunch(value) {
  const params = value instanceof URLSearchParams ? value : new URLSearchParams(value || '');
  const panel = PANELS.has(params.get('panel')) ? params.get('panel') : null;
  const file = cleanPath(params.get('file'));
  const model = MODEL.test(params.get('model') || '') ? params.get('model') : null;
  const entity = ENTITY.test(params.get('entity') || '') ? params.get('entity') : null;
  const variant = VARIANT.test(params.get('variant') || '') ? params.get('variant') : null;
  const lighting = ['off', 'ambient', 'directional'].includes(params.get('lighting')) ? params.get('lighting') : null;
  const gizmos = ['0', '1'].includes(params.get('gizmos')) ? params.get('gizmos') === '1' : null;
  if (!panel && !file && !model && !entity) return null;
  return { version: 1, panel: panel || ((model || entity) ? 'model-preview' : 'source'), file,
    preview: model ? { model, variant } : entity ? { entity } : null,
    controls: { lighting, gizmos } };
}

export function legacyWorkshopUrl(location, kind) {
  const source = new URL(location.href);
  const target = new URL('workshop.html', source);
  const launch = new URLSearchParams();
  if (kind === 'viewer') {
    const model = source.searchParams.get('model');
    const entity = source.searchParams.get('entity');
    launch.set('panel', 'model-preview');
    if (MODEL.test(model || '')) launch.set('model', model);
    if (ENTITY.test(entity || '')) launch.set('entity', entity);
    for (const key of ['variant', 'lighting', 'gizmos']) {
      const value = source.searchParams.get(key); if (value != null) launch.set(key, value);
    }
  } else {
    launch.set('panel', 'files');
    const file = cleanPath(source.searchParams.get('file'));
    if (file) launch.set('file', file);
  }
  launch.set('from', kind);
  target.hash = launch.toString();
  return target.href;
}
