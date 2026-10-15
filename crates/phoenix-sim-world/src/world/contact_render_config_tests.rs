use super::*;

#[test]
fn authored_style_roundtrips_and_old_render_profiles_keep_defaults() {
    let render: crate::world::config::RenderConfig = toml::from_str("hdr = true").unwrap();
    assert_eq!(render.contact_proxy, ContactProxyConfig::default());
    let render: crate::world::config::RenderConfig = toml::from_str(
        "[contact_proxy]\nminimum_radius = 7\ncolour = [0.1, 0.2, 0.3]\nradar_icon = 'contact'",
    )
    .unwrap();
    assert_eq!(render.contact_proxy.minimum_radius, 7.0);
    assert_eq!(render.contact_proxy.colour, [0.1, 0.2, 0.3]);
    let encoded = toml::to_string(&render).unwrap();
    assert_eq!(
        toml::from_str::<crate::world::config::RenderConfig>(&encoded).unwrap(),
        render
    );
}

#[test]
fn invalid_proxy_style_is_refused_at_authoring_decode() {
    for source in [
        "minimum_radius = 0",
        "minimum_radius = -1",
        "minimum_radius = nan",
        "colour = [0, 1, inf]",
        "colour = [-0.1, 0, 1]",
        "colour = [0, 0, 1.1]",
        "radar_icon = ''",
        "unknown = 1",
    ] {
        assert!(
            toml::from_str::<ContactProxyConfig>(source).is_err(),
            "{source}"
        );
    }
}
