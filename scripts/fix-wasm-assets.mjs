import { execSync } from "child_process";
import { mkdtempSync, readFileSync, writeFileSync, copyFileSync, rmSync, existsSync } from "fs";
import { join } from "path";
import { tmpdir } from "os";

const ROOT = "assets/models";
const KTX2_BIN = join(process.cwd(), "node_modules", "ktx2tools", "bin", "windows");
process.env.PATH = `${KTX2_BIN};${process.env.PATH}`;

const ASTEROID_NAMES = [
  "asteroid_common_1",
  "asteroid_common_2",
  "asteroid_common_3",
  "asteroid_common_4",
];

const VARIANTS = ["small", "large", "cosmetic"];

function run(desc, cmd) {
  console.log(`  ${desc}...`);
  execSync(cmd, { stdio: "inherit" });
}

// ── Fix 1: Generate missing LOD sidecar TOML files ──────────────────────────
// Each LOD model shares the same base rig as the main variant.
console.log("\n=== Fix 1: Generating missing LOD sidecar TOML files ===");

let sidecarCount = 0;
for (const name of ASTEROID_NAMES) {
  for (const variant of VARIANTS) {
    const baseSidecar = `${ROOT}/${name}.${variant}.toml`;
    if (!existsSync(baseSidecar)) {
      console.log(`  WARN: base sidecar missing: ${baseSidecar}`);
      continue;
    }
    const template = readFileSync(baseSidecar, "utf8");

    for (const lod of ["lod1", "lod2"]) {
      const lodPath = `${ROOT}/${name}_${lod}.${variant}.toml`;
      if (!existsSync(lodPath)) {
        writeFileSync(lodPath, template);
        console.log(`  created: ${lodPath}`);
        sidecarCount++;
      }
    }
  }
}
console.log(`  ✓ ${sidecarCount} sidecar files generated`);

// ── Fix 2: Decompress KTX2 textures from asteroid GLBs ─────────────────────
console.log("\n=== Fix 2: Decompressing KTX2 textures ===");

const glbFiles = [];
for (const name of ASTEROID_NAMES) {
  glbFiles.push(`${ROOT}/${name}.glb`);
  glbFiles.push(`${ROOT}/${name}_lod1.glb`);
  glbFiles.push(`${ROOT}/${name}_lod2.glb`);
}

for (const glb of glbFiles) {
  const tmp = mkdtempSync(join(tmpdir(), "ktx-decomp-"));
  const tmpGlb = join(tmp, "decomp.glb");
  try {
    run(`${glb} → decompress`, `npx @gltf-transform/cli ktxdecompress "${glb}" "${tmpGlb}"`);
    copyFileSync(tmpGlb, glb);
    console.log(`  ✓ replaced: ${glb}`);
  } finally {
    rmSync(tmp, { recursive: true });
  }
}

console.log("\n=== Done ===");
