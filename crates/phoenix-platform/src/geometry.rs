//! Physical monitor and pane geometry, independent of display roles.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorGeometry {
    pub physical_width: u32,
    pub physical_height: u32,
    pub position_x: i32,
    pub position_y: i32,
    pub scale_factor: f64,
}
/// How a two-pane Station divides its monitor.
///
/// One-pane Stations ignore it (the pane is the whole monitor). The default is
/// [`SideBySide`](PaneSplit::SideBySide), matching the left-to-right tiling the
/// pre-#1123 pane host already used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneSplit {
    /// Two panes left and right, each half the width, full height.
    #[default]
    SideBySide,
    /// Two panes top and bottom, each half the height, full width.
    Stacked,
}

/// One pane's rectangle within its Station window, in physical pixels, origin
/// top-left of the monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Tile `count` panes across `geometry` along `split`, so they exactly cover the
/// monitor with no gap and no overlap.
///
/// One pane is the whole monitor; two divide it in half along the split axis,
/// the second pane absorbing an odd pixel so the two rectangles still tile
/// exactly. `count` is only ever 1 or 2 in a validated profile
/// (the application density policy); the general tiling handles any `count` so this
/// stays one code path rather than a pair of special cases that could drift.
pub fn pane_rects(geometry: &MonitorGeometry, split: PaneSplit, count: usize) -> Vec<PaneRect> {
    if count == 0 {
        return Vec::new();
    }
    let (w, h) = (geometry.physical_width, geometry.physical_height);
    let mut rects = Vec::with_capacity(count);
    let n = count as u32;
    match split {
        PaneSplit::SideBySide => {
            let base = w / n;
            let mut x = 0u32;
            for i in 0..n {
                let width = if i == n - 1 { w - x } else { base };
                rects.push(PaneRect {
                    x,
                    y: 0,
                    width,
                    height: h,
                });
                x += width;
            }
        }
        PaneSplit::Stacked => {
            let base = h / n;
            let mut y = 0u32;
            for i in 0..n {
                let height = if i == n - 1 { h - y } else { base };
                rects.push(PaneRect {
                    x: 0,
                    y,
                    width: w,
                    height,
                });
                y += height;
            }
        }
    }
    rects
}
