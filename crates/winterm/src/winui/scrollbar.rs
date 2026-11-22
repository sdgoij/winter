//! The pane's scrollbar, drawn over the swap chain.
//!
//! The fractions and prompt marks come from `scrollbar` and the snapshot, so
//! this module only turns them into rectangles: a dark track, a tick per
//! prompt, and the thumb. The overlay takes no pointer events of its own; the
//! pane's handlers own the strip (ADR 018).

use windows_reactor::{Canvas, Color, HorizontalAlignment, Rectangle, VerticalAlignment, View};

use crate::scrollbar;
use crate::terminal_render::TerminalData;

/// The track's width in DIPs, matching the egui shell's strip.
pub(super) const WIDTH: f64 = scrollbar::WIDTH as f64;

/// The height of a prompt tick, in DIPs.
const MARK_HEIGHT: f64 = 1.5;

/// The fill behind the thumb.
const TRACK: Color = Color::argb(90, 0, 0, 0);
/// The thumb itself.
const THUMB: Color = Color::argb(200, 160, 160, 160);
/// A prompt that succeeded.
const MARK: Color = Color::argb(220, 110, 170, 255);
/// A prompt whose command failed.
const ERROR: Color = Color::argb(235, 235, 90, 90);

/// Whether `x` lands in the strip on a pane `width` DIPs wide.
pub(super) fn contains(width: f64, x: f64) -> bool {
    x >= width - WIDTH && x <= width
}

/// The top of a tick centred on `position`, kept inside the track.
pub(super) fn mark_top(position: f32, track: f64) -> f64 {
    let top = f64::from(position) * track - MARK_HEIGHT / 2.0;
    top.clamp(0.0, (track - MARK_HEIGHT).max(0.0))
}

/// The bar as an overlay, or `None` when the document fits the pane.
pub(super) fn overlay(scroll: i32, data: &TerminalData, visible: i32, track: f64) -> Option<View> {
    let document = data.document_height();
    if document <= visible || track <= 0.0 {
        return None;
    }

    let mut children: Vec<View> = Vec::with_capacity(data.marks().len() + 2);
    children.push(
        Rectangle::new()
            .fill(TRACK)
            .width(WIDTH)
            .height(track)
            .canvas_left(0.0)
            .canvas_top(0.0)
            .into(),
    );

    for mark in data.marks() {
        children.push(
            Rectangle::new()
                .fill(if mark.error { ERROR } else { MARK })
                .width(WIDTH)
                .height(MARK_HEIGHT)
                .canvas_left(0.0)
                .canvas_top(mark_top(scrollbar::position(mark.row, document), track))
                .into(),
        );
    }

    let (top, height) = scrollbar::thumb_pixels(scroll, document, visible, track);
    children.push(
        Rectangle::new()
            .fill(THUMB)
            .width(WIDTH - 2.0)
            .height(height)
            .radius_x(2.0)
            .radius_y(2.0)
            .canvas_left(1.0)
            .canvas_top(top)
            .into(),
    );

    Some(
        Canvas::new()
            .width(WIDTH)
            .horizontal_alignment(HorizontalAlignment::Right)
            .vertical_alignment(VerticalAlignment::Stretch)
            .children(children)
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strip_is_the_pane_s_right_edge() {
        assert!(contains(100.0, 92.0));
        assert!(contains(100.0, 100.0));
        assert!(!contains(100.0, 85.0));
    }

    #[test]
    fn a_tick_is_centred_on_its_row_and_stays_inside_the_track() {
        assert_eq!(mark_top(0.0, 100.0), 0.0);
        assert_eq!(mark_top(0.5, 100.0), 49.25);
        assert_eq!(mark_top(1.0, 100.0), 98.5);
    }
}
