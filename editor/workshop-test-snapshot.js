import { createStoreZip, crc32 } from './mod-pack-export.js';

/** The adapter supplies captured dependencies, including all render support.
 * It must refuse missing declared bytes; this merger has no fetch fallback. */
export function createWorkshopTestPreparation({ runtime, dependencies }) {
  const encode = new TextEncoder();
  const textFiles = files => Object.fromEntries(Object.entries(files).filter(([path, source]) =>
    typeof source === 'string' && /\.(toml|rhai)$/.test(path)));
  const refused = report => {
    const error = new Error('Runtime validation refused Workshop Test'); error.report = report; throw error;
  };
  function merge(source, files) {
    const merged = { ...source.base_files };
    const appendAssets = assets => {
      for (const [path, bytes] of Object.entries(assets || {})) merged[path] = bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes);
    };
    appendAssets(source.base_assets);
    for (const pack of source.packs || []) { Object.assign(merged, pack.files); appendAssets(pack.assets); }
    Object.assign(merged, files);
    return merged;
  }
  return {
    async catalog(files) {
      const source = await runtime.dependencies();
      return runtime.testCatalog(textFiles(merge(source, files)));
    },
    async prepare(files, selection) {
      const archive = createStoreZip(Object.entries(files).map(([path, source]) => ({ path,
        bytes: typeof source === 'string' ? encode.encode(source) : source })));
      const report = await runtime.validate(archive);
      if (!report.accepted) refused(report);
      const captured = merge(await dependencies(), files);
      const selected = await runtime.checkTestSelection(textFiles(captured), selection);
      if (!selected.accepted) refused(selected);
      // A stable display/correlation fingerprint only. Runtime admission and
      // the strict reader consume the actual immutable bytes, never this CRC.
      const members = Object.entries(captured).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([path, source]) => {
        const bytes = typeof source === 'string' ? encode.encode(source) : source;
        return [path, bytes.length, crc32(bytes)];
      });
      const revision = crc32(encode.encode(JSON.stringify(members))).toString(16).padStart(8, '0');
      return { files: captured, selection, revision };
    },
  };
}
