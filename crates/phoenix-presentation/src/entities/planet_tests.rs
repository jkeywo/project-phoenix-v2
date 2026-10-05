use super::*;
use crate::entities::config::{PlanetAtmosphereConfig, PlanetCloudsConfig, PlanetSurfaceConfig};

fn full_config() -> PlanetConfig {
    PlanetConfig {
        radius: 20.0,
        longitude_segments: 64,
        latitude_segments: 32,
        surface: PlanetSurfaceConfig {
            natural: None,
            city: None,
            albedo: "assets/planets/earth/albedo.webp".into(),
            normal: Some("assets/planets/earth/normal.webp".into()),
            roughness: Some("assets/planets/earth/roughness.webp".into()),
            emissive_colour: Some("assets/planets/earth/emissive_colour.webp".into()),
            emissive_mask: None,
            emissive_night_only: true,
            emissive_strength: 1.5,
        },
        clouds: Some(PlanetCloudsConfig {
            dynamics: None,
            smog: None,
            albedo: "assets/planets/earth/cloud_albedo.webp".into(),
            opacity: Some("assets/planets/earth/cloud_opacity.webp".into()),
            normal: None,
            scale: 1.03,
            drift_speed: 0.0,
        }),
        atmosphere: Some(PlanetAtmosphereConfig {
            scattering: None,
            colour: [0.35, 0.55, 1.0],
            strength: 1.0,
        }),
    }
}

#[test]
fn planet_texture_paths_enumerates_all_declared_maps_with_srgb_flags() {
    let paths = planet_texture_paths(&full_config());
    assert_eq!(
        paths,
        vec![
            ("assets/planets/earth/albedo.webp".to_string(), true),
            ("assets/planets/earth/normal.webp".to_string(), false),
            ("assets/planets/earth/roughness.webp".to_string(), false),
            (
                "assets/planets/earth/emissive_colour.webp".to_string(),
                true
            ),
            ("assets/planets/earth/cloud_albedo.webp".to_string(), true),
            ("assets/planets/earth/cloud_opacity.webp".to_string(), false),
        ]
    );
}

#[test]
fn planet_texture_paths_minimal_config_is_albedo_only() {
    let mut config = full_config();
    config.surface.normal = None;
    config.surface.roughness = None;
    config.surface.emissive_colour = None;
    config.clouds = None;
    let paths = planet_texture_paths(&config);
    assert_eq!(
        paths,
        vec![("assets/planets/earth/albedo.webp".to_string(), true)]
    );
}

#[test]
fn ecumenopolis_preloads_every_shell_and_packed_data_map() {
    let entity = crate::entities::config::EntityConfig::from_toml(include_str!(
        "../../../../assets/entities/planet_ecumenopolis.toml"
    ))
    .unwrap();
    let cfg = entity.planet.unwrap();
    assert!(cfg.surface.city.is_some());
    let paths = planet_texture_paths(&cfg);
    for (suffix, srgb) in [
        ("city_lights.ktx2", false),
        ("city_material.ktx2", false),
        ("smog_normal.ktx2", false),
        ("city_glow.ktx2", true),
        ("optical_depth.png", false),
        ("haze.ktx2", false),
    ] {
        assert!(
            paths
                .iter()
                .any(|(p, colour)| p.ends_with(suffix) && *colour == srgb),
            "missing or incorrectly interpreted {suffix}"
        );
    }
    for (p, _) in &paths {
        assert!(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(p)
                .exists(),
            "missing {p}"
        );
    }
    assert!(!paths.iter().any(|(p, _)| p.ends_with("skyglow.ktx2")));
    let mut with_skyglow = cfg.clone();
    with_skyglow
        .atmosphere
        .as_mut()
        .unwrap()
        .scattering
        .as_mut()
        .unwrap()
        .skyglow = Some("optional-skyglow.ktx2".into());
    assert!(planet_texture_paths(&with_skyglow).contains(&("optional-skyglow.ktx2".into(), true)));
}
