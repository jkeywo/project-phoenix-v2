import { describe, expect, it, vi } from 'vitest';
import { createWorkshopRuntime } from '../../editor/workshop-runtime.js';

describe('offline Workshop runtime capability', () => {
  it('passes exact candidate bytes and an isolated source snapshot without booting a host', async () => {
    const source = { base_files: { 'assets/scenarios.toml': '[content]\nid="base"\nepoch=1' }, packs: [] };
    const validate = vi.fn(() => JSON.stringify({ accepted: true, findings: [] }));
    const boot = vi.fn();
    const load = vi.fn(async () => ({ wasm_workshop_validate_pack: validate, wasm_init: boot, wasm_add_mod_pack: boot }));
    const runtime = createWorkshopRuntime({ load, dependencies: async () => source });
    const bytes = new Uint8Array([0, 25, 255]);
    expect(await runtime.validate(bytes)).toEqual({ accepted: true, findings: [] });
    source.base_files.extra = 'later mutation';
    await runtime.validate(bytes);
    expect(validate).toHaveBeenLastCalledWith(bytes, JSON.stringify({ base_files: { 'assets/scenarios.toml': '[content]\nid="base"\nepoch=1' }, packs: [] }));
    expect(load).toHaveBeenCalledOnce();
    expect(boot).not.toHaveBeenCalled();
  });

  it('never treats missing or contradictory validation as acceptance and permits retry', async () => {
    const load = vi.fn().mockRejectedValueOnce(new Error('missing artifact'))
      .mockResolvedValue({ wasm_workshop_validate_pack: () => JSON.stringify({ accepted: true,
        findings: [{ severity: 'error', message: 'invalid script', file: 's.rhai' }] }) });
    const runtime = createWorkshopRuntime({ load, dependencies: async () => ({ base_files: {} }) });
    await expect(runtime.validate(new Uint8Array())).rejects.toThrow('missing artifact');
    await expect(runtime.validate(new Uint8Array())).rejects.toThrow('conflicting validation');
  });

  it('obtains field metadata and source patches from the same runtime without a JS serializer', async () => {
    const fields = [{ path: ['global', 'title'], kind: 'string', source: "'A'", line: 2 }];
    const patch = vi.fn(() => "# intact\n[global]\ntitle = 'B'\n");
    const runtime = createWorkshopRuntime({ load: async () => ({
      wasm_workshop_validate_pack: () => '{}', wasm_workshop_fields: () => JSON.stringify(fields),
      wasm_workshop_patch: patch,
    }), dependencies: async () => ({ base_files: {} }) });
    const source = "# intact\n[global]\ntitle = 'A'\n";
    expect(await runtime.inspect(source, 'assets/worlds/test.toml')).toEqual(fields);
    const change = { document_path: 'assets/worlds/test.toml', path: fields[0].path, expected_source: source, value_source: "'B'" };
    expect(await runtime.patch(source, change)).toContain("title = 'B'");
    expect(patch).toHaveBeenCalledWith(source, JSON.stringify(change));
  });
});
