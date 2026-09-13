import { expect, it } from 'vitest';
import { assetDependencies } from '../../editor/asset-dependencies.js';

function model(document) {
  const json = new TextEncoder().encode(JSON.stringify({ asset: { version: '2.0' }, ...document }));
  const bytes = new Uint8Array(20 + json.length), view = new DataView(bytes.buffer);
  view.setUint32(0, 0x46546c67, true); view.setUint32(4, 2, true);
  view.setUint32(8, bytes.length, true); view.setUint32(12, json.length, true);
  view.setUint32(16, 0x4e4f534a, true); bytes.set(json, 20); return bytes;
}

it('plans external model bytes at their portable relative paths and ignores embedded data', () => {
  const bytes = model({ buffers: [{ uri: 'vertices.bin' }], images: [{ uri: '../texture.png' }, { uri: 'data:image/png;base64,AAAA' }] });
  expect(assetDependencies('assets/models/ship/probe.glb', bytes)).toEqual(['assets/models/ship/vertices.bin', 'assets/models/texture.png']);
});

it('plans planet descriptors from the asset root and never requests escaped or remote content', () => {
  const bytes = new TextEncoder().encode(JSON.stringify({ source: 'textures/a.ktx2', fallback: 'textures/b.ktx2' }));
  expect(assetDependencies('assets/planets/test.ptex', bytes)).toEqual(['assets/textures/a.ktx2', 'assets/textures/b.ktx2']);
  for (const uri of ['https://example.invalid/a.bin', '../../../escape.bin', '%2e%2e/a.bin']) {
    expect(assetDependencies('assets/models/a.glb', model({ buffers: [{ uri }] }))).toEqual([]);
  }
});
