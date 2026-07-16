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

// Ensure toktx is on PATH for the uastc command
const KTX2_BIN = join(process.cwd(), "node_modules", "ktx2tools", "bin", "windows");
process.env.PATH = `${KTX2_BIN};${process.env.PATH}`;

function run(desc, cmd) {
  console.log(`  ${desc}...`);
  execSync(cmd, { stdio: "inherit" });
}

for (const name of MODELS) {
  const input = `${ROOT}/${name}.glb`;
  const tmp = mkdtempSync(join(tmpdir(), `asteroid-lod-${name}-`));

  console.log(`\n${name}:`);

  // ── Base: KTX2 (1024×1024 originals → UASTC) ────────────────────────────
  run("base → KTX2",
    `npx @gltf-transform/cli uastc "${input}" "${join(tmp, "base.glb")}" --level 2`);

  // ── LOD1: simplify 25% (error 0.01) → resize 512 → KTX2 ────────────────
  run("LOD1 → simplify (25%, err=0.01)",
    `npx @gltf-transform/cli simplify "${input}" "${join(tmp, "lod1_simp.glb")}" --ratio 0.25 --error 0.01`);
  run("LOD1 → resize (512)",
    `npx @gltf-transform/cli resize "${join(tmp, "lod1_simp.glb")}" "${join(tmp, "lod1_rz.glb")}" --width 512 --height 512`);
  run("LOD1 → KTX2",
    `npx @gltf-transform/cli uastc "${join(tmp, "lod1_rz.glb")}" "${ROOT}/${name}_lod1.glb" --level 2`);

  // ── LOD2: simplify 5% (error 0.1) → resize 256 → KTX2 ─────────────────
  run("LOD2 → simplify (5%, err=0.1)",
    `npx @gltf-transform/cli simplify "${input}" "${join(tmp, "lod2_simp.glb")}" --ratio 0.05 --error 0.1`);
  run("LOD2 → resize (256)",
    `npx @gltf-transform/cli resize "${join(tmp, "lod2_simp.glb")}" "${join(tmp, "lod2_rz.glb")}" --width 256 --height 256`);
  run("LOD2 → KTX2",
    `npx @gltf-transform/cli uastc "${join(tmp, "lod2_rz.glb")}" "${ROOT}/${name}_lod2.glb" --level 2`);

  // ── Replace original with KTX2 base ─────────────────────────────────────
  copyFileSync(join(tmp, "base.glb"), input);

  rmSync(tmp, { recursive: true });
  console.log(`  ✓ done (${name})`);
}
