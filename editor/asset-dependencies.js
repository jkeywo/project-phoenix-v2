/** Fetch planning only. The shared Rust validator still admits the bytes. */
export function assetDependencies(path, bytes) {
  try {
    bytes = bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes);
    let uris;
    if (path.endsWith('.glb')) {
      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
      if (view.getUint32(0, true) !== 0x46546c67 || view.getUint32(16, true) !== 0x4e4f534a) return [];
      const data = JSON.parse(new TextDecoder().decode(bytes.subarray(20, 20 + view.getUint32(12, true))));
      uris = [...(data.buffers || []), ...(data.images || [])].map(item => item.uri);
    } else if (path.endsWith('.ptex')) {
      const data = JSON.parse(new TextDecoder().decode(bytes));
      uris = [data.source, data.fallback];
      path = 'assets/descriptor';
    } else return [];
    return [...new Set(uris.filter(uri => typeof uri === 'string' && !uri.startsWith('data:')).map(uri => {
      if (!uri || /[\\:%?#\x00-\x1f\x7f-\x9f]/.test(uri)) throw Error('Non-local reference');
      const parts = path.split('/').slice(0, -1);
      for (const part of uri.split('/')) {
        if (!part) throw Error('Empty reference component');
        if (part === '..') { if (parts.length <= 1) throw Error('Outside content'); parts.pop(); }
        else if (part !== '.') parts.push(part);
      }
      return parts.join('/');
    }))];
  } catch { return []; }
}
