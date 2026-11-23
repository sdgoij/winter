//! `RenderEngineBase` — the behaviours shared by every engine.
//!
//! Ported from `RenderEngineBase.hpp/.cpp`. A concrete engine embeds one of
//! these and delegates the matching trait methods to it, which is how a Rust
//! port of an abstract base class shares the title and frame bookkeeping.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::render_engine::RenderResult;

/// The title and frame state every engine shares.
#[derive(Debug, Clone, Default)]
pub struct RenderEngineBase {
    title_changed: bool,
    last_frame_title: String,
}

impl RenderEngineBase {
    /// Creates an empty base.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a proposed title, flagging it if it differs from the last frame.
    pub fn invalidate_title(&mut self, proposed: &str) {
        if proposed != self.last_frame_title {
            self.title_changed = true;
        }
    }

    /// Applies a title through `do_update` when it differs from the last frame.
    ///
    /// Returns `Ok(true)` when the engine updated, `Ok(false)` when the title
    /// was unchanged. A failed update leaves the recorded title untouched.
    pub fn update_title(
        &mut self,
        new_title: &str,
        do_update: impl FnOnce(&str) -> RenderResult<()>,
    ) -> RenderResult<bool> {
        if new_title == self.last_frame_title {
            return Ok(false);
        }

        do_update(new_title)?;
        self.last_frame_title = new_title.to_string();
        self.title_changed = false;
        Ok(true)
    }

    /// Returns whether a title change is pending for this frame.
    pub const fn title_changed(&self) -> bool {
        self.title_changed
    }

    /// Returns the title the last successful update recorded.
    pub fn last_frame_title(&self) -> &str {
        &self.last_frame_title
    }

    /// Does nothing; a new-text notification is optional.
    pub const fn notify_new_text(&self) -> bool {
        false
    }

    /// Does nothing; soft fonts are optional.
    pub const fn update_soft_font(&self) -> bool {
        false
    }

    /// Does nothing; frame info is optional.
    pub const fn prepare_render_info(&self) -> bool {
        false
    }

    /// Does nothing; line transforms are optional.
    pub const fn reset_line_transform(&self) -> bool {
        false
    }

    /// Does nothing; line transforms are optional.
    pub const fn prepare_line_transform(&self) -> bool {
        false
    }

    /// Does nothing; image slices are optional.
    pub const fn paint_image_slice(&self) -> bool {
        false
    }

    /// No engine needs continuous redraw unless it opts in.
    pub const fn requires_continuous_redraw(&self) -> bool {
        false
    }

    /// Throttles the render loop to roughly 60 FPS and honours shutdown.
    pub fn wait_until_can_render(&self, shutdown: &AtomicBool) -> bool {
        if shutdown.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(8));
        !shutdown.load(Ordering::Relaxed)
    }

    /// Records a hovered hyperlink id; the default base ignores it.
    pub const fn update_hyperlink_hovered_id(&self, _hovered_id: u16) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_engine::RenderError;

    #[test]
    fn a_changed_title_is_flagged_then_applied() {
        let mut base = RenderEngineBase::new();
        base.invalidate_title("winter");
        assert!(base.title_changed());

        let mut applied = String::new();
        let updated = base
            .update_title("winter", |title| {
                applied = title.to_string();
                Ok(())
            })
            .expect("title update");
        assert!(updated);
        assert_eq!(applied, "winter");
        assert_eq!(base.last_frame_title(), "winter");
        assert!(!base.title_changed());
    }

    #[test]
    fn an_unchanged_title_is_not_applied() {
        let mut base = RenderEngineBase::new();
        base.update_title("same", |_| Ok(())).expect("first update");

        let updated = base
            .update_title("same", |_| panic!("must not run"))
            .expect("second update");
        assert!(!updated);
    }

    #[test]
    fn a_failed_update_keeps_the_recorded_title() {
        let mut base = RenderEngineBase::new();
        base.update_title("old", |_| Ok(())).expect("first update");

        let result = base.update_title("new", |_| Err(RenderError::new("nope")));
        assert!(result.is_err());
        assert_eq!(base.last_frame_title(), "old");
    }

    #[test]
    fn the_base_declines_optional_features() {
        let base = RenderEngineBase::new();
        assert!(!base.notify_new_text());
        assert!(!base.update_soft_font());
        assert!(!base.prepare_render_info());
        assert!(!base.reset_line_transform());
        assert!(!base.prepare_line_transform());
        assert!(!base.paint_image_slice());
        assert!(!base.requires_continuous_redraw());
    }

    #[test]
    fn a_signalled_shutdown_stops_the_wait() {
        let base = RenderEngineBase::new();
        let shutdown = AtomicBool::new(true);
        assert!(!base.wait_until_can_render(&shutdown));
    }
}
