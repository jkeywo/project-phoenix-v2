use super::*;

#[test]
fn a_wheel_notch_scrolls_a_line_of_pixels_not_one_pixel() {
    assert_eq!(
        PaneInput::scroll_from_wheel(true, 0.0, -1.0),
        PaneInput::Scroll {
            dx: 0,
            dy: -(WHEEL_LINE_PIXELS as i32)
        }
    );
    assert_eq!(
        PaneInput::scroll_from_wheel(true, 0.0, 2.5),
        PaneInput::Scroll { dx: 0, dy: 150 }
    );
}

#[test]
fn pixel_reports_pass_through_and_sub_notch_ticks_survive_rounding() {
    assert_eq!(
        PaneInput::scroll_from_wheel(false, 12.0, -37.0),
        PaneInput::Scroll { dx: 12, dy: -37 }
    );
    assert_eq!(
        PaneInput::scroll_from_wheel(true, 0.0, 0.1),
        PaneInput::Scroll { dx: 0, dy: 6 }
    );
}
