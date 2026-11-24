//! The tab list and its active index, and the strip that shows it.
//!
//! Toolkit-free (ADR 018): the window draws the titles and turns clicks and
//! chords into commands; this type owns the list and keeps the active index
//! valid through adds, closes and wraps, and [`strip_layout`] owns the rule for
//! where a tab sits so every shell puts it in the same place.

/// The narrowest a tab is drawn, so a title still reads.
pub const TAB_MIN: i32 = 96;
/// The widest a tab is drawn; the strip leaves the rest empty.
pub const TAB_MAX: i32 = 220;
/// The new-tab button's width.
pub const NEW_TAB: i32 = 32;
/// The close box at a tab's right end.
pub const CLOSE: i32 = 22;

/// A box in the strip, in pixels; `right` is exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabBox {
    /// The box's left edge.
    pub left: i32,
    /// The box's right edge, exclusive.
    pub right: i32,
}

impl TabBox {
    /// Returns the box's width in pixels.
    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    /// Returns whether `x` falls in the box.
    pub const fn contains(&self, x: i32) -> bool {
        x >= self.left && x < self.right
    }
}

/// Where the strip's tabs and its new-tab button sit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripLayout {
    /// One box per tab, in order.
    pub tabs: Vec<TabBox>,
    /// The new-tab button, after the last tab.
    pub new_tab: TabBox,
}

/// Splits `width` into equal tab boxes and the new-tab button.
///
/// Tabs shrink to [`TAB_MIN`] and grow to [`TAB_MAX`]. A strip too narrow for
/// its tabs keeps the button at its right edge, over the tab that would have
/// covered it, so the button is reachable at any width.
pub fn strip_layout(width: i32, count: usize) -> StripLayout {
    let width = width.max(0);
    let available = (width - NEW_TAB).max(0);
    let tab_width = match i32::try_from(count) {
        Ok(count) if count > 0 => (available / count).clamp(TAB_MIN, TAB_MAX),
        _ => TAB_MAX,
    };

    let tabs = (0..count)
        .map(|index| {
            let left = i32::try_from(index).unwrap_or(i32::MAX) * tab_width;
            TabBox {
                left,
                right: left.saturating_add(tab_width),
            }
        })
        .collect();
    let left = (tab_width * i32::try_from(count).unwrap_or(0)).min(available);

    StripLayout {
        tabs,
        new_tab: TabBox {
            left,
            right: left + NEW_TAB,
        },
    }
}

/// What a press on the strip asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripHit {
    /// Show the tab at this index.
    Tab(usize),
    /// Close the tab at this index.
    Close(usize),
    /// Start a new tab.
    New,
}

/// Maps a press at `x` to what it asked for.
///
/// The new-tab button is tested first: it is drawn last, so where it overlaps
/// a tab it is what the user sees and therefore what the press names.
pub fn strip_hit(layout: &StripLayout, x: i32) -> Option<StripHit> {
    if layout.new_tab.contains(x) {
        return Some(StripHit::New);
    }

    layout.tabs.iter().enumerate().find_map(|(index, tab)| {
        if !tab.contains(x) {
            return None;
        }
        // The close box is the tab's right end, so a press on the title does
        // not hit it.
        if tab.right - x <= CLOSE {
            Some(StripHit::Close(index))
        } else {
            Some(StripHit::Tab(index))
        }
    })
}

/// A list of tabs with one active index.
#[derive(Debug, Clone)]
pub struct Tabs<T> {
    items: Vec<T>,
    active: usize,
}

impl<T> Tabs<T> {
    /// Creates a list whose only item is active.
    pub fn new(first: T) -> Self {
        Self {
            items: vec![first],
            active: 0,
        }
    }

    /// Returns how many tabs there are.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns the active index.
    pub fn index(&self) -> usize {
        self.active
    }

    /// Returns the tab at `index`.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index)
    }

    /// Returns the tab at `index` mutably.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.items.get_mut(index)
    }

    /// Returns the active tab.
    pub fn active(&self) -> Option<&T> {
        self.items.get(self.active)
    }

    /// Returns the active tab mutably.
    pub fn active_mut(&mut self) -> Option<&mut T> {
        self.items.get_mut(self.active)
    }

    /// Appends `item`, makes it active and returns its index.
    pub fn add(&mut self, item: T) -> usize {
        self.items.push(item);
        self.active = self.items.len() - 1;
        self.active
    }

    /// Removes the tab at `index` and keeps the active tab valid.
    ///
    /// Closing a tab before the active one shifts the active index down so the
    /// same tab stays active. Closing the active tab selects the neighbour that
    /// took its place, or the new last tab. Returns whether any tabs remain.
    pub fn close(&mut self, index: usize) -> bool {
        if index >= self.items.len() {
            return !self.items.is_empty();
        }

        self.items.remove(index);

        if self.items.is_empty() {
            self.active = 0;
            return false;
        }

        if index < self.active {
            self.active -= 1;
        } else if index == self.active {
            self.active = self.active.min(self.items.len() - 1);
        }

        true
    }

    /// Activates `index`, if it exists.
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.items.len() {
            return false;
        }
        self.active = index;
        true
    }

    /// Moves the active tab by `delta`, wrapping at both ends.
    pub fn step(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }

        let count = self.items.len() as i32;
        self.active = (self.active as i32 + delta).rem_euclid(count) as usize;
    }

    /// Reorders the items, where `order[new]` is the item's old index.
    ///
    /// The active tab stays active: its new position is looked up. An `order`
    /// that is not a permutation of the current indices is ignored, so a
    /// toolkit that reports a half-finished drag cannot corrupt the list.
    pub fn reorder(&mut self, order: &[usize]) -> bool {
        let count = self.items.len();
        if order.len() != count {
            return false;
        }

        let mut seen = vec![false; count];
        for &index in order {
            if index >= count || seen[index] {
                return false;
            }
            seen[index] = true;
        }

        let active = order.iter().position(|&old| old == self.active).unwrap_or(0);
        let mut slots: Vec<Option<T>> = self.items.drain(..).map(Some).collect();
        self.items = order
            .iter()
            .filter_map(|&old| slots.get_mut(old).and_then(Option::take))
            .collect();
        self.active = active;
        true
    }

    /// Moves the tab at `from` to sit at `to`, keeping the active tab active.
    ///
    /// Returns whether the order changed, so a drag that stays over its own tab
    /// is a no-op. Both indices are positions before the move; the active tab is
    /// looked up again after it, which is what `reorder` already does.
    pub fn move_to(&mut self, from: usize, to: usize) -> bool {
        let count = self.items.len();
        if from >= count || to >= count || from == to {
            return false;
        }

        let mut order: Vec<usize> = (0..count).collect();
        let moved = order.remove(from);
        order.insert(to, moved);
        self.reorder(&order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles<T: Copy>(tabs: &Tabs<T>) -> Vec<T> {
        (0..tabs.len()).filter_map(|index| tabs.get(index).copied()).collect()
    }

    #[test]
    fn a_strip_puts_one_tab_after_another() {
        let layout = strip_layout(900, 2);

        assert_eq!(layout.tabs.len(), 2);
        assert_eq!(
            layout.tabs[0],
            TabBox {
                left: 0,
                right: TAB_MAX
            }
        );
        assert_eq!(layout.tabs[1].left, TAB_MAX);
        assert_eq!(layout.new_tab.left, layout.tabs[1].right);
        assert_eq!(layout.new_tab.width(), NEW_TAB);
    }

    #[test]
    fn tabs_shrink_to_the_minimum_before_the_strip_overflows() {
        let layout = strip_layout(900, 8);
        // 868 available for eight tabs is 108 each, between the bounds.
        assert_eq!(layout.tabs[0].width(), 108);
        assert_eq!(layout.new_tab.left, 8 * 108);

        let narrow = strip_layout(300, 8);
        assert_eq!(narrow.tabs[0].width(), TAB_MIN);
        // The button stays at the right edge, over the tab it covers.
        assert_eq!(narrow.new_tab.right, 300);
    }

    #[test]
    fn a_press_names_the_tab_its_close_box_or_the_button() {
        let layout = strip_layout(900, 2);

        assert_eq!(strip_hit(&layout, 0), Some(StripHit::Tab(0)));
        assert_eq!(strip_hit(&layout, TAB_MAX - 1), Some(StripHit::Close(0)));
        assert_eq!(strip_hit(&layout, TAB_MAX), Some(StripHit::Tab(1)));
        assert_eq!(strip_hit(&layout, 2 * TAB_MAX + 4), Some(StripHit::New));
        assert_eq!(strip_hit(&layout, 2 * TAB_MAX + NEW_TAB), None);
    }

    #[test]
    fn the_new_tab_button_is_reachable_over_an_overflowing_strip() {
        let layout = strip_layout(300, 8);

        assert_eq!(strip_hit(&layout, 290), Some(StripHit::New));
        // Left of the button is still the tab underneath it.
        assert_eq!(strip_hit(&layout, 200), Some(StripHit::Tab(2)));
    }

    #[test]
    fn an_empty_strip_still_offers_the_button() {
        let layout = strip_layout(900, 0);

        assert!(layout.tabs.is_empty());
        assert_eq!(strip_hit(&layout, 4), Some(StripHit::New));
    }

    #[test]
    fn reordering_keeps_the_active_tab_active() {
        let mut tabs = Tabs::new("a");
        tabs.add("b");
        tabs.add("c");
        tabs.select(1);

        // New order: c, b, a.
        assert!(tabs.reorder(&[2, 1, 0]));

        assert_eq!(titles(&tabs), vec!["c", "b", "a"]);
        assert_eq!(tabs.active(), Some(&"b"));
        assert_eq!(tabs.index(), 1);
    }

    #[test]
    fn an_order_that_is_not_a_permutation_is_ignored() {
        let mut tabs = Tabs::new("a");
        tabs.add("b");
        tabs.add("c");

        assert!(!tabs.reorder(&[0, 1]));
        assert!(!tabs.reorder(&[0, 0, 1]));
        assert!(!tabs.reorder(&[0, 1, 3]));
        assert_eq!(titles(&tabs), vec!["a", "b", "c"]);
        assert_eq!(tabs.active(), Some(&"c"));
    }

    #[test]
    fn moving_a_tab_reorders_the_list_and_keeps_it_active() {
        let mut tabs = Tabs::new("a");
        tabs.add("b");
        tabs.add("c");
        tabs.select(1);

        assert!(tabs.move_to(0, 2));
        assert_eq!(titles(&tabs), vec!["b", "c", "a"]);
        // The tab that was active is still the one shown.
        assert_eq!(tabs.active(), Some(&"b"));
        assert_eq!(tabs.index(), 0);

        assert!(tabs.move_to(2, 0));
        assert_eq!(titles(&tabs), vec!["a", "b", "c"]);
        assert_eq!(tabs.active(), Some(&"b"));
        assert_eq!(tabs.index(), 1);
    }

    #[test]
    fn moving_a_tab_onto_itself_or_off_the_end_changes_nothing() {
        let mut tabs = Tabs::new("a");
        tabs.add("b");

        assert!(!tabs.move_to(0, 0));
        assert!(!tabs.move_to(0, 2));
        assert!(!tabs.move_to(2, 0));
        assert_eq!(titles(&tabs), vec!["a", "b"]);
    }

    #[test]
    fn a_new_list_has_one_active_tab() {
        let tabs = Tabs::new("a");
        assert_eq!(tabs.len(), 1);
        assert!(!tabs.is_empty());
        assert_eq!(tabs.active(), Some(&"a"));
    }

    #[test]
    fn adding_a_tab_activates_it() {
        let mut tabs = Tabs::new("a");
        assert_eq!(tabs.add("b"), 1);
        assert_eq!(tabs.active(), Some(&"b"));
        assert_eq!(titles(&tabs), vec!["a", "b"]);
    }

    #[test]
    fn closing_before_the_active_tab_keeps_the_same_tab_active() {
        let mut tabs = Tabs::new("a");
        tabs.add("b");
        tabs.add("c");

        assert!(tabs.close(0));
        assert_eq!(tabs.active(), Some(&"c"));
        assert_eq!(titles(&tabs), vec!["b", "c"]);
    }

    #[test]
    fn closing_the_active_tab_selects_its_neighbour() {
        let mut tabs = Tabs::new("a");
        tabs.add("b");
        tabs.add("c");
        tabs.select(1);

        assert!(tabs.close(1));
        assert_eq!(tabs.active(), Some(&"c"));

        // Closing the last tab selects the new last tab.
        assert!(tabs.close(1));
        assert_eq!(tabs.active(), Some(&"a"));
    }

    #[test]
    fn closing_the_last_tab_empties_the_list() {
        let mut tabs = Tabs::new("a");
        assert!(!tabs.close(0));
        assert!(tabs.is_empty());
        assert_eq!(tabs.active(), None);
        assert_eq!(tabs.index(), 0);
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut tabs = Tabs::new("a");
        tabs.add("b");
        tabs.add("c");
        tabs.select(0);

        tabs.step(-1);
        assert_eq!(tabs.active(), Some(&"c"));
        tabs.step(1);
        assert_eq!(tabs.active(), Some(&"a"));
        tabs.step(5);
        assert_eq!(tabs.active(), Some(&"c"));
    }

    #[test]
    fn selecting_out_of_range_is_ignored() {
        let mut tabs = Tabs::new("a");
        tabs.add("b");

        assert!(!tabs.select(2));
        assert_eq!(tabs.active(), Some(&"b"));
        assert!(tabs.select(0));
        assert_eq!(tabs.active(), Some(&"a"));
    }
}
