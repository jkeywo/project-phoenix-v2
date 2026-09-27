import { describe, expect, it } from 'vitest';
import { createWasmBuildReceipt, verifyWasmBuildReceipt } from '../../scripts/fleet-wasm-build-receipt.mjs';
import { optionsFrom } from '../../scripts/fleet-browser-matrix.mjs';
import { mixedOptions } from '../../scripts/fleet-mixed-matrix.mjs';

const hash = char => char.repeat(64);
const source = { sourceRevision: 'a'.repeat(40), sourcePatch: '' };
const bundle = { 'index.html': hash('b'), 'client/index.html': hash('c'), 'server_bg.wasm': hash('d') };

describe('source-matched browser WASM receipt', () => {
  it('binds a clean source revision to every bundle file and the WASM bytes', () => {
    const receipt = createWasmBuildReceipt(source, bundle);
    expect(receipt.wasmSha256).toEqual({ 'server_bg.wasm': hash('d') });
    expect(verifyWasmBuildReceipt(receipt, source, bundle)).toBe(true);
    expect(() => verifyWasmBuildReceipt(receipt, source, { ...bundle, 'client/index.html': hash('e') })).toThrow('bundle mismatch');
    expect(() => verifyWasmBuildReceipt(receipt, source, { ...bundle, 'extra.js': hash('e') })).toThrow('bundle mismatch');
    expect(() => verifyWasmBuildReceipt(receipt, source, { ...bundle, 'server_bg.wasm': hash('e') })).toThrow('bundle mismatch');
  });
  it('refuses a changed revision, dirty checkout, forged WASM list and incomplete bundle', () => {
    const receipt = createWasmBuildReceipt(source, bundle);
    expect(() => verifyWasmBuildReceipt(receipt, { ...source, sourceRevision: 'f'.repeat(40) }, bundle)).toThrow('revision mismatch');
    expect(() => verifyWasmBuildReceipt(receipt, { ...source, sourcePatch: ' M gui/server.js' }, bundle)).toThrow('clean source');
    expect(() => verifyWasmBuildReceipt({ ...receipt, wasmSha256: {} }, source, bundle)).toThrow('artifact mismatch');
    expect(() => createWasmBuildReceipt(source, { 'client/index.html': hash('c'), 'index.html': hash('b') })).toThrow('no WASM');
  });
  it('keeps browser and native receipt options distinct in both runners', () => {
    expect(optionsFrom(['--out', 'evidence', '--wasm-build-receipt', 'wasm.json'])['wasm-build-receipt']).toMatch(/wasm\.json$/);
    const mixed = mixedOptions(['--out', 'evidence', '--binary', 'native.exe', '--bundle', 'native-dist', '--build-receipt', 'native.json', '--wasm-build-receipt', 'wasm.json']);
    expect(mixed['build-receipt']).toMatch(/native\.json$/);
    expect(mixed['wasm-build-receipt']).toMatch(/wasm\.json$/);
  });
});
