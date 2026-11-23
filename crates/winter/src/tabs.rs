//! The tab list and its active index.
//!
//! Toolkit-free (ADR 018): the window draws the titles and turns clicks and
//! chords into commands; this type owns the list and keeps the active index
//! valid through adds, closes and wraps.

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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles<T: Copy>(tabs: &Tabs<T>) -> Vec<T> {
        (0..tabs.len()).filter_map(|index| tabs.get(index).copied()).collect()
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
