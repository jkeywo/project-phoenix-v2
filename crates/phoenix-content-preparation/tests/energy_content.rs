//! Accepted content must retain a real energy owner after include/instance composition.
use phoenix_content_preparation::entities::{
    config::EntityConfig,
    include_resolve::{resolve_template, FragmentSource, ParseEntityTemplate},
    loader::apply_overrides,
};
use std::path::{Path, PathBuf};

struct Repository(PathBuf);
impl FragmentSource for Repository {
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(path)).ok()
    }
    fn absence_is_final(&self) -> bool {
        true
    }
}
impl Repository {
    fn entity(&self, path: &str) -> EntityConfig {
        resolve_template(path, self)
            .unwrap_or_else(|e| panic!("{path}: {e:?}"))
            .parse()
            .unwrap_or_else(|e| panic!("{path}: {e:?}"))
    }
}
fn toml_files(directory: &Path, output: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().unwrap() != "fragments" {
                toml_files(&path, output);
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "toml")
        {
            output.push(path);
        }
    }
}

#[test]
fn shipped_energy_hardware_survives_templates_and_world_overrides() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let repo = Repository(root.clone());
    let mut files = Vec::new();
    toml_files(&root.join("assets/entities"), &mut files);
    files.sort();
    let mut hardware = 0;
    for file in files {
        let path = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        hardware += usize::from(repo.entity(&path).energy.is_some());
    }
    assert!(
        hardware >= 20,
        "the corpus must exercise NPC and player equipment"
    );
    let mut worlds = Vec::new();
    toml_files(&root.join("assets/worlds"), &mut worlds);
    worlds.sort();
    let mut instances = 0;
    for file in worlds {
        let text = std::fs::read_to_string(&file).unwrap();
        let world = phoenix_content_preparation::world::config::parse_world(&text)
            .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        for entity in world.entities {
            let config = repo.entity(&entity.template_path);
            if let Some(overrides) = entity.overrides {
                apply_overrides(&config, &overrides)
                    .unwrap_or_else(|e| panic!("{} {}: {e}", file.display(), entity.template_path));
            }
            instances += 1;
        }
    }
    assert!(instances > 50);
}
