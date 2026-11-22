//! The pane's scrollbar: where the view sits in the scrollback, and the drag
//! that moves it.
//!
//! The module is toolkit-free (ADR 018): fractions of the track are its whole
//! interface, and `ui.rs` turns them into pixels. Marks come from the snapshot,
//! which already knows the document's rows.

/// How wide the bar is, in pixels.
pub const WIDTH: f32 = 10.0;
/// The smallest the thumb is drawn, in pixels, so it stays grabbable.
pub const MIN_THUMB: f32 = 12.0;

/// What a press on the track asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// The press jumped the view to this many rows above the live screen.
    Jump(i32),
    /// The press grabbed the thumb; a drag now moves it.
    Grab,
}

/// The scrollbar's drag state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Scrollbar {
    /// The pointer's offset inside the thumb while it is held.
    grab: Option<f32>,
}

impl Scrollbar {
    /// Returns whether the thumb is held.
    pub const fn dragging(&self) -> bool {
        self.grab.is_some()
    }

    /// Takes a press at `fraction`, measured from the top of the track.
    pub fn press(&mut self, fraction: f32, scroll: i32, document: i32, visible: i32) -> Press {
        let (top, height) = thumb(scroll, document, visible);
        if fraction >= top && fraction <= top + height {
            self.grab = Some(fraction - top);
            Press::Grab
        } else {
            Press::Jump(scroll_at(fraction - height / 2.0, document, visible))
        }
    }

    /// Continues a drag at `fraction`, or `None` when the thumb is not held.
    pub fn drag(&mut self, fraction: f32, document: i32, visible: i32) -> Option<i32> {
        let grab = self.grab?;
        Some(scroll_at(fraction - grab, document, visible))
    }

    /// Ends a drag.
    pub fn release(&mut self) {
        self.grab = None;
    }
}

/// The thumb's top and height as fractions of the track.
///
/// The whole document maps to the track, so the thumb reaches the top at the
/// oldest row and its bottom edge at the live screen.
pub fn thumb(scroll: i32, document: i32, visible: i32) -> (f32, f32) {
    let scrollable = document - visible;
    if scrollable <= 0 || document <= 0 {
        return (0.0, 1.0);
    }

    let top = (scrollable - scroll).clamp(0, scrollable) as f32 / document as f32;
    (top, visible as f32 / document as f32)
}

/// The scroll a thumb top names.
pub fn scroll_at(top: f32, document: i32, visible: i32) -> i32 {
    let scrollable = (document - visible).max(0);
    if scrollable == 0 || document <= 0 {
        return 0;
    }

    let top = top.clamp(0.0, 1.0);
    let row = (top * document as f32).round() as i32;
    (document - visible - row).clamp(0, scrollable)
}

/// Where `row` sits on the track, from 0 at the top to 1 at the bottom.
pub fn position(row: i32, document: i32) -> f32 {
    if document <= 0 {
        return 0.0;
    }
    (row as f32 / document as f32).clamp(0.0, 1.0)
}

/// The index of the tick a press at `fraction` lands on.
///
/// `tolerance` is how far from a tick, as a fraction of the track, still
/// counts. When two ticks are within reach, the nearer one wins.
pub fn tick_at(positions: &[f32], fraction: f32, tolerance: f32) -> Option<usize> {
    positions
        .iter()
        .enumerate()
        .filter(|(_, position)| (fraction - **position).abs() <= tolerance)
        .min_by(|(_, found), (_, other)| (fraction - **found).abs().total_cmp(&(fraction - **other).abs()))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thumb_sits_at_the_bottom_on_the_live_screen() {
        assert_eq!(thumb(0, 100, 20), (0.8, 0.2));
        assert_eq!(thumb(40, 100, 20), (0.4, 0.2));
        assert_eq!(thumb(80, 100, 20), (0.0, 0.2));

        // Without history the thumb fills the track.
        assert_eq!(thumb(0, 20, 20), (0.0, 1.0));
    }

    #[test]
    fn a_thumb_top_names_a_scroll() {
        assert_eq!(scroll_at(0.8, 100, 20), 0, "the live screen");
        assert_eq!(scroll_at(0.4, 100, 20), 40);
        assert_eq!(scroll_at(0.0, 100, 20), 80, "the oldest row");

        // A thumb pushed past either end stops at it.
        assert_eq!(scroll_at(0.9, 100, 20), 0);
        assert_eq!(scroll_at(0.0, 20, 20), 0);
    }

    #[test]
    fn a_press_on_the_thumb_grabs_it_and_a_drag_moves_the_view() {
        let mut bar = Scrollbar::default();
        assert_eq!(bar.press(0.9, 0, 100, 20), Press::Grab);
        assert!(bar.dragging());

        // The press landed 0.1 into the thumb, so dragging to 0.5 moves the
        // top to 0.4.
        assert_eq!(bar.drag(0.5, 100, 20), Some(40));

        bar.release();
        assert!(!bar.dragging());
        assert_eq!(bar.drag(0.5, 100, 20), None);
    }

    #[test]
    fn a_press_above_the_thumb_jumps_towards_the_history() {
        let mut bar = Scrollbar::default();
        assert_eq!(bar.press(0.4, 0, 100, 20), Press::Jump(50));
        assert!(!bar.dragging());
    }

    #[test]
    fn a_row_maps_onto_the_track() {
        assert_eq!(position(0, 100), 0.0);
        assert_eq!(position(50, 100), 0.5);
        assert_eq!(position(200, 100), 1.0);
        assert_eq!(position(5, 0), 0.0);
    }

    #[test]
    fn a_press_lands_on_the_nearest_tick_within_reach() {
        let ticks = [0.1, 0.5, 0.9];
        assert_eq!(tick_at(&ticks, 0.12, 0.05), Some(0));
        assert_eq!(tick_at(&ticks, 0.48, 0.05), Some(1));

        // Two ticks in reach: the nearer one wins.
        assert_eq!(tick_at(&[0.50, 0.52], 0.505, 0.05), Some(0));

        // A press between ticks, and an empty track, find nothing.
        assert_eq!(tick_at(&ticks, 0.3, 0.05), None);
        assert_eq!(tick_at(&[], 0.3, 0.05), None);
    }
}
