// Local launcher for the built Workshop; no downloaded server or WASM required.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('../dist/', import.meta.url));
const host = '127.0.0.1';
const port = 8083;
const url = `http://${host}:${port}/workshop.html`;
const types = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.csv': 'text/csv; charset=utf-8',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.webp': 'image/webp',
  '.woff2': 'font/woff2',
  '.ttf': 'font/ttf',
};

const server = createServer(async (req, res) => {
  if (req.method !== 'GET' && req.method !== 'HEAD') {
    res.writeHead(405, { Allow: 'GET, HEAD' }).end();
    return;
  }
  try {
    const pathname = decodeURIComponent(new URL(req.url, url).pathname);
    const file = path.resolve(root, `.${pathname === '/' ? '/workshop.html' : pathname}`);
    const relative = path.relative(root, file);
    if (relative.startsWith('..') || path.isAbsolute(relative)) {
      res.writeHead(403).end();
      return;
    }
    const content = await readFile(file);
    res.writeHead(200, {
      'Content-Type': types[path.extname(file)] || 'application/octet-stream',
      'Content-Length': content.length,
      'Cache-Control': 'no-store',
    });
    res.end(req.method === 'HEAD' ? undefined : content);
  } catch {
    res.writeHead(404).end();
  }
});

server.on('error', error => {
  console.error(`[run-workshop] ${error.code === 'EADDRINUSE'
    ? `Port ${port} is already in use. Close the other server and run again.`
    : error.message}`);
  process.exitCode = 1;
});

server.listen(port, host, () => {
  console.log(`[run-workshop] ${url}\nKeep this window open. Press Ctrl+C to stop.`);
  if (!process.argv.includes('--no-open') && process.platform === 'win32') {
    // Open only after listening, so the browser never races the build/server.
    const opener = spawn('powershell.exe', ['-NoProfile', '-Command', `Start-Process '${url}'`], {
      windowsHide: true, stdio: 'ignore',
    });
    opener.on('error', () => console.error(`[run-workshop] Open ${url} in your browser.`));
    opener.on('exit', code => {
      if (code) console.error(`[run-workshop] Open ${url} in your browser.`);
    });
  }
});
