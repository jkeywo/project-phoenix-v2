import { execSync } from "child_process";
import { mkdtempSync, copyFileSync, rmSync } from "fs";
import { join } from "path";
import { tmpdir } from "os";

const ROOT = "assets/models";
const MODELS = [
  "asteroid_common_1",
  "asteroid_common_2",
  "asteroid_common_3",
  "asteroid_common_4",
];

function run(desc, cmd) {
  console.log(`  ${desc}...`);
  execSync(cmd, { stdio: "inherit" });
}

for (const name of MODELS) {
  const input = `${ROOT}/${name}.glb`;
  const tmp = mkdtempSync(join(tmpdir(), `asteroid-lod-${name}-`));

  console.log(`\n${name}:`);

  // ── Base: copy as-is (no KTX2 — WASM can't decode BasisU) ──────────────
  copyFileSync(input, join(tmp, "base.glb"));

  // ── LOD1: simplify 25% (error 0.01) → resize 512 ───────────────────────
  run("LOD1 → simplify (25%, err=0.01)",
    `npx @gltf-transform/cli simplify "${input}" "${join(tmp, "lod1_simp.glb")}" --ratio 0.25 --error 0.01`);
  run("LOD1 → resize (512)",
    `npx @gltf-transform/cli resize "${join(tmp, "lod1_simp.glb")}" "${ROOT}/${name}_lod1.glb" --width 512 --height 512`);

  // ── LOD2: simplify 5% (error 0.1) → resize 256 ─────────────────────────
  run("LOD2 → simplify (5%, err=0.1)",
    `npx @gltf-transform/cli simplify "${input}" "${join(tmp, "lod2_simp.glb")}" --ratio 0.05 --error 0.1`);
  run("LOD2 → resize (256)",
    `npx @gltf-transform/cli resize "${join(tmp, "lod2_simp.glb")}" "${ROOT}/${name}_lod2.glb" --width 256 --height 256`);

  // ── Regenerate sidecar TOML files for the new LOD GLBs ──────────────────
  // LOD models share the same base rig as the original variant.

  rmSync(tmp, { recursive: true });
  console.log(`  ✓ done (${name})`);
}
