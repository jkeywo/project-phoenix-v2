/** Offline runtime capability. Loading the local WASM module does not start
 * the simulation. Every validation call gets an explicit immutable dependency
 * bundle and the exact candidate bytes, never a live host overlay. */
import { createWorkshopAssetSnapshot } from './workshop-assets.js';

/** The dependency bundle with every byte member stripped: base files plus each
 * pack's id, manifest and text files. */
export function textDependencies(source) {
  return { base_files: { ...(source?.base_files || {}) },
    packs: (source?.packs || []).map(({ id, manifest_toml, files }) => ({ id, manifest_toml, files: { ...(files || {}) } })) };
}

export function createWorkshopRuntime({
  load = async () => {
    const moduleUrl = new URL('../phoenix.js', import.meta.url).href;
    const module = await import(/* @vite-ignore */ moduleUrl);
    await module.default();
    return module;
  },
  dependencies = async () => {
    const response = await fetch(new URL('../workshop-base.json', import.meta.url));
    if (!response.ok) throw new Error(`Workshop dependencies: HTTP ${response.status}`);
    return response.json();
  },
  fetchAsset,
} = {}) {
  let pending = null;
  async function ready() {
    if (!pending) pending = Promise.all([load(), dependencies()]).then(([runtime, source]) => {
      if (typeof runtime?.wasm_workshop_validate_pack !== 'function' || !source?.base_files) {
        throw new Error('Workshop runtime capability is unavailable');
      }
      return { runtime, dependencies: structuredClone(source), assets: createWorkshopAssetSnapshot(source,
        fetchAsset ? { fetch: fetchAsset } : {}) };
    }).catch(error => { pending = null; throw error; });
    return pending;
  }
  return {
    async dependencies() { return structuredClone((await ready()).dependencies); },
    async testDependencies() { return (await ready()).assets.captureAllBuffers(); },
    async testCatalog(files) {
      const { runtime } = await ready();
      return JSON.parse(runtime.wasm_workshop_test_catalog(JSON.stringify(files)));
    },
    async checkTestSelection(files, selection) {
      const { runtime } = await ready();
      return JSON.parse(runtime.wasm_workshop_test_check_selection(JSON.stringify(files), JSON.stringify({ selection, revision: '' })));
    },
    /** Read-only dependencies for local previews. Editable candidate bytes
     * stay with WorkshopDocument; no live overlay or uncaptured HTTP fallback. */
    async readAsset(path) {
      const snapshot = await (await ready()).assets.captureBuffers([path]);
      for (const pack of [...(snapshot.packs || [])].reverse()) {
        if (Object.hasOwn(pack.assets || {}, path)) return Uint8Array.from(pack.assets[path]);
      }
      return Object.hasOwn(snapshot.base_assets || {}, path)
        ? Uint8Array.from(snapshot.base_assets[path]) : null;
    },
    async inspect(source, documentPath) {
      const { runtime } = await ready();
      return JSON.parse(runtime.wasm_workshop_fields(source, documentPath));
    },
    async patch(source, change) {
      const { runtime } = await ready();
      return runtime.wasm_workshop_patch(source, JSON.stringify(change));
    },
    /** The runtime-derived faction and complexity catalog of the draft's text
     * members (issue #1474). Dependencies travel as TEXT ONLY: the catalog
     * resolves cross-file references by parsing sources, so the asset bytes the
     * validator needs would only be serialised to be ignored. */
    async definitions(files) {
      const { runtime, dependencies } = await ready();
      return JSON.parse(runtime.wasm_workshop_definitions(JSON.stringify(files),
        JSON.stringify(textDependencies(dependencies))));
    },
    /** All-or-nothing structural edits over exact source; the runtime answers
     * with the whole new document or refuses without touching anything. */
    async edit(source, request) {
      const { runtime } = await ready();
      return runtime.wasm_workshop_edit(source, JSON.stringify(request));
    },
    /** A faction skeleton spelled by the runtime type, never a JS template. */
    async newFaction(name, uuid) {
      const { runtime } = await ready();
      return runtime.wasm_workshop_new_faction(name, uuid);
    },
    /** The runtime-derived composition catalog of the draft's text members
     * (issue #1475): the manifest's roots, every world's extra worlds and
     * script-driven references with their origins, the members, the choices,
     * what Test and the lobby would list, and the findings. Text-only
     * dependencies, as the definition catalog takes: origins and cycles are
     * resolved by parsing sources. */
    async composition(files) {
      const { runtime, dependencies } = await ready();
      return JSON.parse(runtime.wasm_workshop_composition(JSON.stringify(files),
        JSON.stringify(textDependencies(dependencies))));
    },
    /** Structural edits over one member checked against the whole candidate
     * and its dependencies: the runtime answers with the new source or refuses
     * a missing, cyclic, duplicate or disallowed reference with the source
     * untouched. */
    async compose(files, request) {
      const { runtime, dependencies } = await ready();
      return runtime.wasm_workshop_compose(JSON.stringify(files), JSON.stringify(textDependencies(dependencies)),
        JSON.stringify(request));
    },
    /** A world skeleton spelled by the runtime type, never a JS template. */
    async newWorld(title) {
      const { runtime } = await ready();
      return runtime.wasm_workshop_new_world(title);
    },
    /** The runtime-derived composition of ONE entity template (issue #1476):
     * its includes with their origins, the merge order, which components are
     * local or inherited, every effective field with the member that authored
     * it, the components the runtime type supports, the fragments it could
     * still include, and the findings. Text-only dependencies, as the other two
     * catalogs take: the resolver reads sources. */
    async entity(files, path) {
      const { runtime, dependencies } = await ready();
      return JSON.parse(runtime.wasm_workshop_entity(JSON.stringify(files),
        JSON.stringify(textDependencies(dependencies)), path));
    },
    /** Structural edits over one template checked against the whole candidate
     * and its dependencies: the runtime answers with the new source or refuses a
     * missing, cyclic, self or disallowed include, an unsupported component or a
     * template that would stop parsing, with the source untouched. */
    async editEntity(files, request) {
      const { runtime, dependencies } = await ready();
      return runtime.wasm_workshop_entity_edit(JSON.stringify(files),
        JSON.stringify(textDependencies(dependencies)), JSON.stringify(request));
    },
    /** The one place a resolved runtime VALUE becomes source: the runtime reads
     * the value at that provenance address and writes it into the local
     * document as new text, leaving every other byte alone (criterion 2). */
    async materialiseEntity(files, path, address) {
      const { runtime, dependencies } = await ready();
      return runtime.wasm_workshop_entity_materialise(JSON.stringify(files),
        JSON.stringify(textDependencies(dependencies)), path, address);
    },
    async validate(bytes) {
      const loaded = await ready();
      const required = loaded.runtime.wasm_workshop_asset_dependencies?.(bytes) || [];
      const snapshot = await loaded.assets.capture(required);
      const report = JSON.parse(loaded.runtime.wasm_workshop_validate_pack(bytes, JSON.stringify(snapshot)));
      if (typeof report?.accepted !== 'boolean' || !Array.isArray(report.findings)
          || report.findings.some(finding => !['error', 'warning'].includes(finding.severity)
            || typeof finding.message !== 'string' || typeof finding.file !== 'string')) {
        throw new Error('Workshop runtime returned an invalid validation report');
      }
      if (report.accepted && report.findings.some(finding => finding.severity === 'error')) {
        throw new Error('Workshop runtime returned conflicting validation status');
      }
      return report;
    },
  };
}
