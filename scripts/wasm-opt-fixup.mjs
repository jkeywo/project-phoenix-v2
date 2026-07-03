// Trunk post_build hook (see Trunk.toml).
//
// This machine/CI's Rust toolchain emits `bulk-memory` and
// `nontrapping-float-to-int` WASM instructions by default for
// wasm32-unknown-unknown (some from our own code, some baked into the
// precompiled std/core that ships with the toolchain). Trunk's bundled
// wasm-opt validates against a stricter feature set with no way to pass
// `--enable-*` flags through Trunk config, so `data-wasm-opt` on the
// `<link data-trunk rel="rust">` tag in server.html is disabled and this
// script runs the same -Oz size optimization directly via the `binaryen`
// npm package, which lets us enable those two features for validation.
//
// Only runs for release builds (TRUNK_BUILD_RELEASE=true) — skipped during
// `trunk serve`/dev builds so hot-reload iteration stays fast.
import { readdirSync, readFileSync, writeFileSync, statSync } from "node:fs";
import { join } from "node:path";
import binaryen from "binaryen";

const isRelease = process.env.TRUNK_BUILD_RELEASE === "true";
if (!isRelease) {
  process.exit(0);
}

// wasm-bindgen names output files with a content hash, and stale hashes
// from previous builds aren't cleaned up here, so pick the most recently
// written one rather than an arbitrary directory-order match.
const distDir = process.argv[2] ?? "dist";
const wasmFile = readdirSync(distDir)
  .filter((f) => f.endsWith("_bg.wasm"))
  .map((f) => ({ f, mtime: statSync(join(distDir, f)).mtimeMs }))
  .sort((a, b) => b.mtime - a.mtime)[0]?.f;
if (!wasmFile) {
  console.error(`wasm-opt-fixup: no *_bg.wasm found in ${distDir}`);
  process.exit(1);
}

const path = join(distDir, wasmFile);
const before = statSync(path).size;
const bytes = readFileSync(path);

const features =
  binaryen.Features.MVP |
  binaryen.Features.BulkMemory |
  binaryen.Features.BulkMemoryOpt |
  binaryen.Features.NontrappingFPToInt;

const module = binaryen.readBinaryWithFeatures(bytes, features);
module.setFeatures(features);
binaryen.setOptimizeLevel(2);
binaryen.setShrinkLevel(2); // -Oz
module.optimize();
const output = module.emitBinary();
module.dispose();

writeFileSync(path, output);

const fmt = (n) => (n / 1024 / 1024).toFixed(2);
console.log(
  `wasm-opt-fixup: ${wasmFile} ${fmt(before)}MB -> ${fmt(output.length)}MB`,
);
