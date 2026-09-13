//! Packaged non-speech definitions. Pure validation; no live data or dispatch.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const PATH: &str = "assets/audio/sound-cues.toml";
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub file: String,
    pub category: String,
    pub informative: bool,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Equivalent {
    pub meaning: String,
    pub source: String,
    pub urgency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bearing: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevation: Option<f32>,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SoundDefinition {
    pub id: String,
    pub label: String,
    pub file: String,
    pub category: String,
    pub audience: String,
    pub volume: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equivalent: Option<Equivalent>,
}
impl SoundDefinition {
    /// Authored listener-relative direction. This never consults hidden truth.
    pub fn position(&self) -> Option<[f32; 3]> {
        let equivalent = self.equivalent.as_ref()?;
        let angle = equivalent.bearing?.to_radians();
        let pitch = equivalent.elevation.unwrap_or(0.0).to_radians();
        Some([
            crate::simmath::sin(angle) * crate::simmath::cos(pitch),
            crate::simmath::sin(pitch),
            -crate::simmath::cos(angle) * crate::simmath::cos(pitch),
        ])
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub version: u32,
    pub assets: Vec<Asset>,
    pub cues: Vec<SoundDefinition>,
}
pub fn bundled() -> Catalog {
    toml::from_str(include_str!("../assets/audio/sound-cues.toml")).expect("sound-cue-catalog")
}
fn text(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().count() <= 512
}
fn asset_path(file: &str) -> bool {
    file.starts_with("assets/sounds/")
        && file
            .rsplit_once('.')
            .is_some_and(|(_, extension)| matches!(extension, "mp3" | "ogg" | "wav"))
        && file.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part.contains(['\\', ':', '%', '?', '#', '<', '>', '"', '|', '*'])
                && !part.chars().any(char::is_control)
        })
}
impl Catalog {
    pub fn validate(&self, cue: &SoundDefinition) -> Result<(), &'static str> {
        if !text(&cue.id)
            || !cue
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || cue.id.starts_with('-')
            || !text(&cue.label)
        {
            return Err("definition");
        }
        let asset = self
            .assets
            .iter()
            .find(|asset| asset.file == cue.file)
            .ok_or("asset")?;
        if !asset_path(&cue.file) {
            return Err("asset");
        }
        if !matches!(
            cue.category.as_str(),
            "music" | "ambience" | "effects" | "alerts" | "interface"
        ) || asset.category != cue.category
        {
            return Err("category");
        }
        if !matches!(cue.audience.as_str(), "viewscreen" | "station" | "gm")
            || (cue.audience != "viewscreen"
                && !matches!(cue.category.as_str(), "alerts" | "interface"))
        {
            return Err("audience");
        }
        if !cue.volume.is_finite() || !(0.0..=1.0).contains(&cue.volume) {
            return Err("volume");
        }
        let Some(equivalent) = &cue.equivalent else {
            return if asset.informative || matches!(cue.category.as_str(), "alerts" | "effects") {
                Err("equivalent")
            } else {
                Ok(())
            };
        };
        if !text(&equivalent.meaning)
            || !text(&equivalent.source)
            || !matches!(
                equivalent.urgency.as_str(),
                "info" | "advisory" | "warning" | "critical"
            )
        {
            return Err("equivalent");
        }
        if equivalent
            .bearing
            .is_some_and(|v| !v.is_finite() || v.abs() > 180.0)
            || equivalent
                .elevation
                .is_some_and(|v| !v.is_finite() || v.abs() > 90.0 || equivalent.bearing.is_none())
        {
            return Err("direction");
        }
        Ok(())
    }
    pub fn validate_all(&self) -> Result<(), &'static str> {
        let mut files = BTreeSet::new();
        let floors = bundled().assets;
        if self.version != 1
            || self.cues.len() > 128
            || self.assets.len() > 128
            || self.assets.iter().any(|asset| {
                !asset_path(&asset.file)
                    || !files.insert(&asset.file)
                    || floors.iter().any(|known| {
                        known.file == asset.file
                            && (known.category != asset.category
                                || (known.informative && !asset.informative))
                    })
                    || !matches!(
                        asset.category.as_str(),
                        "music" | "ambience" | "effects" | "alerts" | "interface"
                    )
            })
        {
            return Err("catalog");
        }
        let mut seen = BTreeSet::new();
        for cue in &self.cues {
            if !seen.insert(&cue.id) {
                return Err("duplicate");
            }
            self.validate(cue)?;
        }
        Ok(())
    }
}

/// Parse authored definitions and their metadata before resolving any bytes.
pub fn parse_source(source: &str) -> Result<Catalog, String> {
    let catalog: Catalog = toml::from_str(source).map_err(|error| error.to_string())?;
    catalog.validate_all().map_err(str::to_string)?;
    Ok(catalog)
}

/// The caller supplies candidate/active/base availability after its asset gate;
/// selected projects supply only their captured source set.
pub fn validate_source(
    source: &str,
    asset_available: impl Fn(&str) -> bool,
) -> Result<Catalog, String> {
    let catalog = parse_source(source)?;
    for cue in &catalog.cues {
        if !asset_available(&cue.file) {
            return Err(format!("asset: {}", cue.file));
        }
    }
    Ok(catalog)
}

/// Local audition carries one asset declaration with its definition. Known
/// packaged sounds retain their category/information floors; new local assets
/// still pass through the production decoder before any samples are played.
pub fn validate_definition(
    cue: &SoundDefinition,
    asset: Option<Asset>,
) -> Result<(), &'static str> {
    let asset = asset
        .or_else(|| {
            bundled()
                .assets
                .into_iter()
                .find(|asset| asset.file == cue.file)
        })
        .ok_or("asset")?;
    let catalog = Catalog {
        version: 1,
        assets: vec![asset],
        cues: vec![cue.clone()],
    };
    catalog.validate_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Deserialize)]
    struct Case {
        name: String,
        cue: SoundDefinition,
        expected: Option<String>,
    }
    #[test]
    fn shared_validation_examples_pin_metadata_and_audience_rules() {
        let catalog = bundled();
        catalog.validate_all().unwrap();
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../tests/fixtures/sound-cue-validation.json"))
                .unwrap();
        for case in cases {
            assert_eq!(
                catalog.validate(&case.cue).err(),
                case.expected.as_deref(),
                "{}",
                case.name
            );
        }
    }
    #[test]
    fn authored_catalog_requires_real_resolved_assets_and_immutable_information_floors() {
        let source = include_str!("../assets/audio/sound-cues.toml");
        let available: BTreeSet<_> = bundled()
            .assets
            .into_iter()
            .map(|asset| asset.file)
            .collect();
        assert!(validate_source(source, |path| available.contains(path)).is_ok());
        assert!(validate_source(source, |path| path != "assets/sounds/Blaster.mp3").is_err());
        let changed = source.replacen("informative = true", "informative = false", 1);
        assert!(validate_source(&changed, |_| true).is_err());
        let unknown = source.replace("volume = 1.0", "volume = 1.0\nspeech = true");
        assert!(validate_source(&unknown, |_| true).is_err());
    }
    #[test]
    fn new_nested_packaged_sound_does_not_require_stock_inventory_and_missing_bytes_refuse() {
        let source = r#"version=1
[[assets]]
file="assets/sounds/custom/sonar ping.wav"
category="alerts"
informative=true
[[cues]]
id="sonar"
label="Sonar report"
file="assets/sounds/custom/sonar ping.wav"
category="alerts"
audience="station"
volume=0.2
[cues.equivalent]
meaning="Contact report ready"
source="Sensors"
urgency="advisory"
"#;
        let catalog =
            validate_source(source, |path| path == "assets/sounds/custom/sonar ping.wav").unwrap();
        assert!(validate_source(source, |_| false).is_err());
        assert!(validate_definition(&catalog.cues[0], Some(catalog.assets[0].clone())).is_ok());
        assert!(validate_source(&source.replace("custom/", "../"), |_| true).is_err());
    }
}
