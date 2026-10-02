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
#[path = "render_geometry_tests.rs"]
mod tests;
