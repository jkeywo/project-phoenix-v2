use super::*;
#[test]
fn web_overrides_keep_web_defaults_and_leave_native_independent() {
    let render: crate::world::config::RenderConfig =
        toml::from_str("[web]\nflare_intensity = 1.5\nmotes = false").unwrap();
    assert_eq!(render.web.flare_intensity, 1.5);
    assert!(!render.web.motes);
    assert_eq!(render.web.shadow_resolution, 1024);
    assert_eq!(render.web.shadow_distance, 120.0);
    assert_eq!(render.native, NativeRenderConfig::default());
    let old: crate::world::config::RenderConfig = toml::from_str("hdr = true").unwrap();
    assert_eq!(old.web, WebRenderConfig::default());
}
#[test]
fn old_render_tables_adopt_native_defaults_and_allow_independent_overrides() {
    let old: crate::world::config::RenderConfig = toml::from_str("hdr = true").unwrap();
    assert_eq!(old.native.flare_intensity, 3.0);
    assert!(old.native.motes && old.native.star_shadows);
    let custom: crate::world::config::RenderConfig =
        toml::from_str("[native]\nflare_intensity = 0.0\nmotes = false").unwrap();
    assert_eq!(custom.native.flare_intensity, 0.0);
    assert!(!custom.native.motes);
    assert!(custom.native.star_shadows);
    assert_eq!(
        custom.native.mote_count,
        NativeRenderConfig::default().mote_count
    );
}
