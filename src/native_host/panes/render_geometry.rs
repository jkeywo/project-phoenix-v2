//! Display and raster geometry for the opt-in native console experiment (#1405).
//!
//! The compositor and input router retain the physical display rectangle and
//! monitor scale. Only the SDK view and texture use the reduced raster size and
//! device scale. Changing device scale alone would change the page's layout.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaneRenderScale {
    #[default]
    Native,
    Half,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterGeometry {
    pub size: (u32, u32),
    pub device_scale: f64,
}

impl PaneRenderScale {
    pub const fn divisor(self) -> u32 {
        match self {
            Self::Native => 1,
            Self::Half => 2,
        }
    }

    /// Round each raster extent upward to a whole pixel; never create a zero
    /// texture. An odd extent therefore has at most one extra display pixel's
    /// worth of CSS viewport. This is an explicit prototype limitation, not a
    /// claim that odd-size text layout and hit mapping have been judged by eye.
    pub fn geometry(self, display: (u32, u32), window_scale: f64) -> RasterGeometry {
        let divisor = self.divisor();
        RasterGeometry {
            size: (
                display.0.max(1).div_ceil(divisor),
                display.1.max(1).div_ceil(divisor),
            ),
            device_scale: window_scale / f64::from(divisor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_host::bridge_profile::PaneRect;
    use crate::native_host::input_routing::{PanePlacement, PaneRouter, WindowKey};
    use crate::native_host::panes::registry::PaneId;

    #[test]
    fn half_raster_keeps_the_css_extent_and_actual_pointer_coordinates() {
        let display = (1920, 1200);
        let scale = 1.25;
        let native = PaneRenderScale::Native.geometry(display, scale);
        let half = PaneRenderScale::Half.geometry(display, scale);
        assert_eq!(native.size, display);
        assert_eq!(native.device_scale, scale);
        assert_eq!(half.size, (960, 600));
        assert_eq!(half.device_scale, 0.625);
        assert_eq!(half.size.0 as f64 / half.device_scale, 1536.0);
        assert_eq!(half.size.1 as f64 / half.device_scale, 960.0);
        assert_eq!(half.size.0 * half.size.1 * 4, display.0 * display.1);

        // This is the ordinary physical-to-CSS router, using the display's
        // dimensions/scale, as both native and reduced textures must do.
        let router = PaneRouter::new(vec![PanePlacement {
            pane: PaneId(7),
            window: WindowKey(3),
            window_origin_x: -1920,
            window_origin_y: 0,
            rect: PaneRect {
                x: 100,
                y: 50,
                width: display.0,
                height: display.1,
            },
            scale_factor: scale,
        }]);
        let hit = router
            .resolve_in_window(WindowKey(3), 1350.0, 675.0)
            .unwrap();
        assert_eq!(hit.pane, PaneId(7));
        assert_eq!((hit.local_x, hit.local_y), (1000, 500));
        assert_eq!(hit.local_x as f64 * half.device_scale, 625.0);
        assert_eq!(hit.local_y as f64 * half.device_scale, 312.5);
    }

    #[test]
    fn odd_and_minimal_sizes_keep_bounded_nonzero_rasters() {
        let odd = PaneRenderScale::Half.geometry((1921, 1081), 1.5);
        assert_eq!(odd.size, (961, 541));
        assert_eq!(odd.device_scale, 0.75);
        assert_eq!(odd.size.0 * 2 - 1921, 1);
        assert_eq!(odd.size.1 * 2 - 1081, 1);
        assert_eq!(PaneRenderScale::Half.geometry((1, 1), 1.0).size, (1, 1));
        assert_eq!(PaneRenderScale::Half.geometry((0, 0), 1.0).size, (1, 1));
        assert_eq!(
            PaneRenderScale::Half
                .geometry((u32::MAX, u32::MAX), 1.0)
                .size,
            (u32::MAX / 2 + 1, u32::MAX / 2 + 1)
        );
    }
}
