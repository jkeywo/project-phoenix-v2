import { WorkshopDocument } from './workshop-document.js';
import { createBrowserWorkshopProvider } from './workshop-provider.js';
import { parse } from 'smol-toml';

/** Original source packs supply dependencies; the selected pack gets a fresh
 * document/undo stack. Runtime projections are never converted back to TOML. */
export function workshopSourceProvider(source, { load } = {}) {
  const packs = source.archives.map(({ id, bytes }) => {
    const document = new WorkshopDocument(bytes);
    const manifest_toml = document.read('scenarios.toml');
    if (parse(manifest_toml).pack?.id !== id) throw Error('Retained pack identity changed');
    const files = Object.create(null), assets = Object.create(null);
    for (const path of document.paths()) {
      if (path === 'scenarios.toml') continue;
      if (document.isBinary(path)) assets[path] = document.bytes(path);
      else files[path] = document.read(path);
    }
    return { id, manifest_toml, files, assets };
  });
  const selected = source.archives.find(pack => pack.id === source.selectedId);
  if (!selected) throw Error('Selected pack source is unavailable');
  return createBrowserWorkshopProvider({ loadedPack: selected.bytes, dependencies: { ...source.base, packs }, ...(load ? { load } : {}) });
}
