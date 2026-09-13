/** Offline runtime capability. Loading the local WASM module does not start
 * the simulation. Every validation call gets an explicit immutable dependency
 * bundle and the exact candidate bytes, never a live host overlay. */
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
} = {}) {
  let pending = null;
  async function ready() {
    if (!pending) pending = Promise.all([load(), dependencies()]).then(([runtime, source]) => {
      if (typeof runtime?.wasm_workshop_validate_pack !== 'function' || !source?.base_files) {
        throw new Error('Workshop runtime capability is unavailable');
      }
      return { runtime, dependencies: JSON.stringify(source) };
    }).catch(error => { pending = null; throw error; });
    return pending;
  }
  return {
    async dependencies() { return JSON.parse((await ready()).dependencies); },
    async inspect(source, documentPath) {
      const { runtime } = await ready();
      return JSON.parse(runtime.wasm_workshop_fields(source, documentPath));
    },
    async patch(source, change) {
      const { runtime } = await ready();
      return runtime.wasm_workshop_patch(source, JSON.stringify(change));
    },
    async validate(bytes) {
      const loaded = await ready();
      const report = JSON.parse(loaded.runtime.wasm_workshop_validate_pack(bytes, loaded.dependencies));
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
