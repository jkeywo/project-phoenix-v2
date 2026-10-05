import { execFileSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
execFileSync('cargo', ['build', '-p', 'phoenix-grid', '--lib', '--target', 'wasm32-unknown-unknown'], { cwd: root, stdio: 'inherit' });
execFileSync(process.env.GRID_WASM_BINDGEN || 'wasm-bindgen', ['--target', 'web', '--out-dir', 'examples/grid/pkg', 'target/wasm32-unknown-unknown/debug/phoenix_grid.wasm'], { cwd: root, stdio: 'inherit' });
