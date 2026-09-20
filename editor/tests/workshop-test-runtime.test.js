import { describe, expect, it, vi } from 'vitest';
import { launchWorkshopTest } from '../workshop-test-runtime.js';

const world = 'assets/worlds/test.toml', ship = 'assets/entities/hull.toml', fragment = 'assets/entities/fragment.toml';
const rig = 'assets/models/hull.glb.toml';
const snapshot = () => ({ selection: { world, ship, seed: 7 }, revision: 'captured', files: {
  [world]: '[global]\n# unsaved', [ship]: 'includes = ["fragment.toml"]', [fragment]: '# included\r\n',
  'assets/scripts/test.rhai': 'let sample = 4;', 'assets/models/hull.glb': Uint8Array.of(0, 255),
} });
function fixture() {
  let config, content, status = null;
  const runtime = {
    set_config_request_callback: vi.fn(fn => { config = fn; }), set_world_fetch_callback: vi.fn(fn => { content = fn; }),
    wasm_push_world_toml: vi.fn(), wasm_push_sidecar_toml: vi.fn(), wasm_push_scenario_manifest: vi.fn(),
    wasm_load_world: vi.fn(() => { content('assets/scripts/test.rhai'); }),
    wasm_workshop_test_preload_ship: vi.fn(() => { config(fragment); config(rig); }),
    wasm_load_config: vi.fn(), wasm_preload_error: vi.fn(() => ''), wasm_is_preload_complete: vi.fn(() => true),
    wasm_fail_preload_fetch: vi.fn(), wasm_fail_world_fetch: vi.fn(),
    wasm_select_ship: vi.fn(), wasm_validate_stations: vi.fn(),
    wasm_get_gm_role_presets: vi.fn(() => JSON.stringify([{ id: 'draft-gm', label: 'draft.label',
      panels: ['gm-map-panel'], quick_actions: [], contacts: [], widget: [
        { id: 'brief', type: 'note', label: 'draft.brief', text: 'draft.note' },
      ] }])),
    wasm_workshop_test_init: vi.fn(() => { status = { running: true, starting: false, tick: 0, acknowledged: 0 }; }),
    wasm_workshop_test_status: vi.fn(() => status && JSON.stringify(status)),
    wasm_workshop_test_control: vi.fn(json => { const value = JSON.parse(json); status = { ...status, tick: status.tick + 1, acknowledged: value.id }; }),
  };
  return { runtime, load: async () => runtime };
}

describe('captured browser Test boot', () => {
  it('loads the selected hull/include/optional rig and Rhai through normal callbacks using only captured text', async () => {
    const { runtime, load } = fixture();
    const captured = snapshot();
    const onGmRolePresets = vi.fn();
    const run = await launchWorkshopTest(captured, { load, onGmRolePresets });
    expect(runtime.wasm_load_world).toHaveBeenCalledWith(world, captured.files[world], [ship]);
    expect(runtime.wasm_load_config).toHaveBeenCalledExactlyOnceWith(fragment, '# included\r\n');
    expect(runtime.wasm_push_sidecar_toml).toHaveBeenCalledWith(rig, '');
    expect(runtime.wasm_push_world_toml).toHaveBeenCalledWith('assets/scripts/test.rhai', 'let sample = 4;');
    expect(runtime.wasm_select_ship).toHaveBeenCalledWith(ship);
    expect(runtime.wasm_validate_stations).toHaveBeenCalledWith(ship, captured.files[ship]);
    expect(runtime.wasm_workshop_test_init).toHaveBeenCalledWith(JSON.stringify({ selection: captured.selection, revision: captured.revision }), captured.files);
    expect(onGmRolePresets).toHaveBeenCalledExactlyOnceWith(runtime.wasm_get_gm_role_presets());
    expect(runtime.wasm_workshop_test_control).not.toHaveBeenCalled();
    expect(await run.control({ command: 'step' })).toMatchObject({ tick: 1, acknowledged: 1 });
    run.dispose(); await expect(run.status()).rejects.toThrow('closed');
  });

  it('refuses missing required captured includes before app creation', async () => {
    const { runtime, load } = fixture(); const captured = snapshot(); delete captured.files[fragment];
    await expect(launchWorkshopTest(captured, { load })).rejects.toThrow(`unavailable: ${fragment}`);
    expect(runtime.wasm_fail_preload_fetch).toHaveBeenCalledWith(fragment, expect.stringContaining('unavailable'));
    expect(runtime.wasm_workshop_test_init).not.toHaveBeenCalled();
  });

  it('propagates real boot failures but permits Winit’s documented control-flow unwind', async () => {
    const { runtime, load } = fixture();
    runtime.wasm_workshop_test_init.mockImplementationOnce(() => { throw new Error('GPU device lost'); });
    await expect(launchWorkshopTest(snapshot(), { load })).rejects.toThrow('GPU device lost');
    const second = fixture(); const init = second.runtime.wasm_workshop_test_init.getMockImplementation();
    second.runtime.wasm_workshop_test_init.mockImplementationOnce(() => { init(); throw new Error('Using exceptions for control flow'); });
    const run = await launchWorkshopTest(snapshot(), { load: second.load });
    expect(await run.status()).toMatchObject({ running: true }); run.dispose();
  });

  it('does not start a late-loaded WASM module after the iframe is retired', async () => {
    const { runtime } = fixture(); const abort = new AbortController(); let finish; const onGmRolePresets = vi.fn();
    const launch = launchWorkshopTest(snapshot(), { load: () => new Promise(resolve => { finish = resolve; }),
      signal: abort.signal, onGmRolePresets });
    abort.abort(); finish(runtime);
    await expect(launch).rejects.toThrow();
    expect(runtime.set_config_request_callback).not.toHaveBeenCalled();
    expect(runtime.wasm_workshop_test_init).not.toHaveBeenCalled();
    expect(onGmRolePresets).not.toHaveBeenCalled();
  });

  it('rejects a retired boot before stale role descriptors can reach the page', async () => {
    const { runtime, load } = fixture(); const abort = new AbortController(); const onGmRolePresets = vi.fn();
    runtime.wasm_workshop_test_init.mockImplementation(() => {});
    const launch = launchWorkshopTest(snapshot(), { load, signal: abort.signal, onGmRolePresets });
    await vi.waitFor(() => expect(runtime.wasm_workshop_test_init).toHaveBeenCalled());
    abort.abort();
    await expect(launch).rejects.toThrow();
    expect(onGmRolePresets).not.toHaveBeenCalled();
  });
});
