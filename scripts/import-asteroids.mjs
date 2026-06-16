import fs from "fs";
import path from "path";

const root = process.cwd();

function readGlbBounds(glbPath) {
  const buf = fs.readFileSync(glbPath);
  const jsonLen = buf.readUInt32LE(12);
  const json = JSON.parse(buf.toString("utf8", 20, 20 + jsonLen));
  const posAccessor = json.accessors.find((a) => a.type === "VEC3" && a.min && a.max);
  return { min: posAccessor.min, max: posAccessor.max };
}

function maxHorizontal(bounds) {
  const { min, max } = bounds;
  return Math.max(Math.abs(min[0]), Math.abs(max[0]), Math.abs(min[2]), Math.abs(max[2]));
}

function writeModelToml(destTomlPath, scale, offsetY, bounds) {
  const { min, max } = bounds;
  const exMin = [min[0] * scale, min[1] * scale + offsetY, min[2] * scale];
  const exMax = [max[0] * scale, max[1] * scale + offsetY, max[2] * scale];
  const size = [exMax[0] - exMin[0], exMax[1] - exMin[1], exMax[2] - exMin[2]];
  const toml = `[base]
offset = [ 0, ${offsetY}, 0 ]
rotation = [ 0, 0, 0 ]
scale = [ ${scale}, ${scale}, ${scale} ]

[extents]
min = [ ${exMin[0]}, ${exMin[1]}, ${exMin[2]} ]
max = [ ${exMax[0]}, ${exMax[1]}, ${exMax[2]} ]
size = [ ${size[0]}, ${size[1]}, ${size[2]} ]

[markers]
`;
  fs.writeFileSync(destTomlPath, toml);
}

const variants = [
  { suffix: "small", radius: 2.0, hull: 30, colour: [0.55, 0.50, 0.42] },
  { suffix: "large", radius: 4.0, hull: 100, colour: [0.60, 0.55, 0.45] },
];
const cosmetic = { radius: 1.0, colour: [0.50, 0.45, 0.38] };

const gameplayPaths = [];
const cosmeticPaths = [];

for (let n = 1; n <= 4; n++) {
  const rawDir = path.join(root, "raw", "models", `PPAsteroidCommon${n}`);
  const rawGlb = path.join(rawDir, "base_basic_pbr.glb");
  const bounds = readGlbBounds(rawGlb);
  const horiz = maxHorizontal(bounds);

  const modelName = `asteroid_common_${n}`;

  for (const v of variants) {
    const scale = v.radius / horiz;
    const offsetY = -(scale * bounds.max[1]) / 2;
    const modelTomlName = `${modelName}_${v.suffix}`;
    writeModelToml(
      path.join(root, "assets", "models", `${modelTomlName}.model.toml`),
      scale,
      offsetY,
      bounds
    );

    const entityName = `asteroid_common_${n}_${v.suffix}`;
    const entityToml = `name = "${v.suffix === "small" ? "Small" : "Large"} Asteroid ${n}"
tags = ["asteroid", "gameplay", "${v.suffix}"]

[target]
tags = ["asteroid"]
threat_level = "none"

[collider]
shape = "Ball"
radius = ${v.radius}
length = 0.0

[hull]
hull_integrity = ${v.hull}

[mesh]
model = "assets/models/${modelTomlName}.glb"
shape = "sphere"
radius = ${v.radius}
colour = [${v.colour.join(", ")}]
`;
    fs.writeFileSync(path.join(root, "assets", "entities", `${entityName}.toml`), entityToml);
    gameplayPaths.push(`assets/entities/${entityName}.toml`);
  }

  // cosmetic (scaled down) version
  const cScale = cosmetic.radius / horiz;
  const cOffsetY = -(cScale * bounds.max[1]) / 2;
  const cosmeticModelTomlName = `${modelName}_cosmetic`;
  writeModelToml(
    path.join(root, "assets", "models", `${cosmeticModelTomlName}.model.toml`),
    cScale,
    cOffsetY,
    bounds
  );

  const cosmeticEntityName = `asteroid_common_${n}_cosmetic`;
  const cosmeticEntityToml = `name = "Cosmetic Asteroid ${n}"
tags = ["asteroid", "cosmetic"]

[collider]
shape = "Ball"
radius = ${cosmetic.radius}
length = 0.0

[mesh]
model = "assets/models/${cosmeticModelTomlName}.glb"
shape = "sphere"
radius = ${cosmetic.radius}
colour = [${cosmetic.colour.join(", ")}]
`;
  fs.writeFileSync(path.join(root, "assets", "entities", `${cosmeticEntityName}.toml`), cosmeticEntityToml);
  cosmeticPaths.push(`assets/entities/${cosmeticEntityName}.toml`);

  // each variant references the same source mesh but gets its own model.toml (different scale/offset)
  fs.copyFileSync(rawGlb, path.join(root, "assets", "models", `${modelName}_small.glb`));
  fs.copyFileSync(rawGlb, path.join(root, "assets", "models", `${modelName}_large.glb`));
  fs.copyFileSync(rawGlb, path.join(root, "assets", "models", `${modelName}_cosmetic.glb`));
}

console.log("gameplayPaths", gameplayPaths);
console.log("cosmeticPaths", cosmeticPaths);

function updateField(fieldFile) {
  const p = path.join(root, "assets", "entities", fieldFile);
  let txt = fs.readFileSync(p, "utf8");
  const gpBlock = `asteroid_type_paths = [\n${gameplayPaths.map((x) => `    "${x}",`).join("\n")}\n]`;
  const cosBlock = `cosmetic_type_paths = [\n${cosmeticPaths.map((x) => `    "${x}",`).join("\n")}\n]`;
  txt = txt.replace(/asteroid_type_paths = \[[^\]]*\]/, gpBlock);
  txt = txt.replace(/cosmetic_type_paths = \[[^\]]*\]/, cosBlock);
  fs.writeFileSync(p, txt);
}

updateField("asteroid_field_main.toml");
updateField("asteroid_belt_axiom.toml");
