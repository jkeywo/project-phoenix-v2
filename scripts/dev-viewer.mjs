// Dev launcher for the model viewer (npm run dev:viewer).
//
// Starts two things:
//   1. A tiny static server for ./assets on ASSET_PORT.
//   2. `trunk serve` on 8081, proxying /assets/* to (1).
//
// Why not Trunk's own `rel="copy-dir"`, as server.html uses? The assets tree is
// ~300 MB, and Trunk's staging rename of it fails on Windows with "Access is
// denied" (os error 5) whenever the output directory is fresh. Proxying avoids
// the copy entirely, which also means an edited .wgsl or .glb is live on the
// next page load rather than after a 300 MB restage.

import { createServer } from 'node:http';
import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import path from 'node:path';

const ASSET_PORT = 8082;
const ASSETS_ROOT = path.join(process.cwd(), 'assets');

const MIME = {
  '.glb': 'model/gltf-binary',
  '.gltf': 'model/gltf+json',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
  '.ktx2': 'image/ktx2',
  '.toml': 'text/plain; charset=utf-8',
  '.json': 'application/json',
  '.wgsl': 'text/plain; charset=utf-8',
  '.ogg': 'audio/ogg',
  '.mp3': 'audio/mpeg',
  '.ttf': 'font/ttf',
};

const assetServer = createServer(async (req, res) => {
  // Strip the /assets prefix Trunk's proxy forwards, and the query string.
  const urlPath = decodeURIComponent(req.url.split('?')[0]).replace(/^\/assets\/?/, '');
  const filePath = path.join(ASSETS_ROOT, urlPath);

  // Refuse anything that escapes the assets root.
  if (!filePath.startsWith(ASSETS_ROOT)) {
    res.writeHead(403).end('Forbidden');
    return;
  }

  try {
    const stats = await stat(filePath);
    if (!stats.isFile()) throw new Error('not a file');
    res.writeHead(200, {
      'Content-Type': MIME[path.extname(filePath).toLowerCase()] ?? 'application/octet-stream',
      'Content-Length': stats.size,
      // Sidecars and shaders change constantly during iteration.
      'Cache-Control': 'no-cache',
    });
    createReadStream(filePath).pipe(res);
  } catch {
    // 404s are expected and meaningful: a missing rig sidecar tells Rust to
    // fall back to an identity rig rather than retry forever.
    res.writeHead(404).end('Not found');
  }
});

assetServer.listen(ASSET_PORT, () => {
  console.log(`[dev-viewer] assets served from ./assets on :${ASSET_PORT}`);
});

const trunk = spawn(
  'trunk',
  [
    'serve',
    '--config', 'viewer-trunk.toml',
    '--proxy-backend', `http://localhost:${ASSET_PORT}/assets/`,
    '--proxy-rewrite', '/assets/',
  ],
  { stdio: 'inherit', shell: process.platform === 'win32' },
);

const shutdown = () => {
  trunk.kill();
  assetServer.close();
};
process.on('SIGINT', shutdown);
process.on('SIGTERM', shutdown);
trunk.on('exit', (code) => {
  assetServer.close();
  process.exit(code ?? 0);
});
