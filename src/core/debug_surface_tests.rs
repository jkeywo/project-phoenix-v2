use super::*;

#[test]
fn catalogue_order_identity_and_wire_names_are_one_to_one() {
    assert_eq!(DEBUG_SURFACE_CATALOGUE.len(), DebugSurface::ALL.len());
    for (descriptor, surface) in DEBUG_SURFACE_CATALOGUE.iter().zip(DebugSurface::ALL) {
        assert_eq!(descriptor.surface, surface);
        assert_eq!(descriptor.wire_name, surface.wire_name());
        assert_eq!(
            DebugSurface::from_wire_name(descriptor.wire_name),
            Some(surface)
        );
    }
}

#[test]
fn pause_is_not_a_debug_surface_wire_name() {
    assert_eq!(DebugSurface::from_wire_name("Pause"), None);
}
