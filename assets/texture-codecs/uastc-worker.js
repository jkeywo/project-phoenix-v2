/* global BASIS */
// Pinned Basis Universal browser runtime from three@0.180.0 (Apache-2.0).
importScripts('./basis/basis_transcoder.js');
const formats = { astc: 10, bc7: 7, etc2: 1, rgba: 13 };
self.onmessage = async ({ data: { buffer, target } }) => {
  let file;
  try {
    if (!Object.hasOwn(formats, target)) throw new Error('Unsupported UASTC target');
    const module = await BASIS({ locateFile: name => new URL('./basis/' + name, self.location.href).href });
    module.initializeBasis();
    const response = await fetch(new URL('./opaque-4k-templates.json', self.location.href));
    if (!response.ok) throw new Error('UASTC template download failed');
    const template = (await response.json())[target];
    file = new module.KTX2File(new Uint8Array(buffer));
    // The approved Gas Giant, Ice Moon and Ecumenopolis maps share this layout.
    if (!file.isValid() || !file.isUASTC() || file.getWidth() !== 4096 ||
        file.getHeight() !== 2048 || file.getLevels() !== 13 || file.getHasAlpha()) {
      throw new Error('Unsupported planet UASTC layout');
    }
    if (!file.startTranscoding()) throw new Error('Basis startTranscoding failed');
    const output = new Uint8Array(template.length);
    output.set(Uint8Array.from(atob(template.header), ch => ch.charCodeAt(0)));
    for (let mip = 0; mip < 13; mip++) {
      const level = template.levels[mip];
      const length = file.getImageTranscodedSizeInBytes(mip, 0, 0, formats[target]);
      if (length !== level.length) throw new Error(`UASTC mip ${mip} size mismatch`);
      const pixels = new Uint8Array(length);
      if (!file.transcodeImage(pixels, mip, 0, 0, formats[target], 0, -1, -1)) {
        throw new Error(`UASTC mip ${mip} failed`);
      }
      output.set(pixels, level.offset);
    }
    self.postMessage({ buffer: output.buffer }, [output.buffer]);
  } catch (error) {
    self.postMessage({ error: String(error) });
  } finally {
    file?.close();
    file?.delete();
  }
};
