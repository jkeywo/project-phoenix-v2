// @vitest-environment node
import { readFile } from 'node:fs/promises';
import { describe, expect, it } from 'vitest';

const read = path => readFile(new URL(`../../${path}`, import.meta.url), 'utf8');

describe('retired standalone shells', () => {
  for (const [file, mode] of [['editor.html', 'editor'], ['viewer.html', 'viewer']]) {
    it(`${file} is an accessible Workshop redirect only`, async () => {
      const source = await read(file);
      expect(source).toContain(`data-workshop-redirect="${mode}"`);
      expect(source).toContain('gui/workshop-redirect.js');
      expect(source).toContain('workshop.html');
      expect(source).not.toMatch(/data-trunk|editor\/app-v2|viewer_init/);
    });
  }

  it('publishes every redirect dependency with the Workshop bundle', async () => {
    const build = await read('scripts/build-workshop.mjs');
    expect(build).toContain("'workshop-launch'");
    expect(build).toContain("'editor.html'");
    expect(build).toContain("'viewer.html'");
    const redirect = await read('gui/workshop-redirect.js');
    expect(redirect).toContain("../editor/workshop-launch.js");
  });
});
