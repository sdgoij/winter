//! The pane layout: a split tree over panes, and the focus walk.
//!
//! Toolkit-free (ADR 018). The window maps the fractional rectangles to pixels
//! and paints each pane; this module owns the tree, the ratios and the rules
//! for splitting, closing, resizing and moving focus.

/// A stable pane identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneId(pub u64);

/// How a split divides its area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Side by side: the children divide the width.
    Horizontal,
    /// Stacked: the children divide the height.
    Vertical,
}

/// A direction for focus movement and for resizing a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// The smallest and largest share a split gives its first child.
const MIN_RATIO: f32 = 0.1;
const MAX_RATIO: f32 = 0.9;

/// A rectangle in fractions of its parent, all in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fraction {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Fraction {
    /// The whole area.
    pub const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    /// Returns the centre of the rectangle.
    pub fn center(self) -> (f32, f32) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

/// The pane tree.
#[derive(Debug, Clone)]
pub enum Layout {
    /// A single pane fills its area.
    Leaf(PaneId),
    /// Two children divided along `axis`.
    Split {
        axis: Axis,
        /// The first child's share, in `MIN_RATIO..=MAX_RATIO`.
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

impl Layout {
    /// Creates a tree with one pane.
    pub fn leaf(id: PaneId) -> Self {
        Self::Leaf(id)
    }

    /// Returns whether this is a single pane.
    pub fn is_single(&self) -> bool {
        matches!(self, Self::Leaf(_))
    }

    /// Returns this node's pane, if it is a leaf.
    pub fn leaf_id(&self) -> Option<PaneId> {
        match self {
            Self::Leaf(id) => Some(*id),
            Self::Split { .. } => None,
        }
    }

    /// Returns whether `id` is somewhere in the tree.
    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Self::Leaf(leaf) => *leaf == id,
            Self::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    /// Returns the panes in paint order.
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut leaves = Vec::new();
        self.collect_leaves(&mut leaves);
        leaves
    }

    fn collect_leaves(&self, leaves: &mut Vec<PaneId>) {
        match self {
            Self::Leaf(id) => leaves.push(*id),
            Self::Split { first, second, .. } => {
                first.collect_leaves(leaves);
                second.collect_leaves(leaves);
            }
        }
    }

    /// Replaces `target` with a split of it and `new`.
    ///
    /// The target keeps the first half; a fresh split is half and half.
    pub fn split(&mut self, target: PaneId, axis: Axis, new: PaneId) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                let target = *id;
                *self = Self::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Leaf(target)),
                    second: Box::new(Self::Leaf(new)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => first.split(target, axis, new) || second.split(target, axis, new),
        }
    }

    /// Removes `target`, collapsing its parent to the surviving sibling.
    ///
    /// Returns `false` when the target is the only pane: a tab cannot have an
    /// empty layout.
    pub fn remove(&mut self, target: PaneId) -> bool {
        let Self::Split { first, second, .. } = self else {
            return false;
        };

        if first.leaf_id() == Some(target) {
            *self = (**second).clone();
            return true;
        }
        if second.leaf_id() == Some(target) {
            *self = (**first).clone();
            return true;
        }

        first.remove(target) || second.remove(target)
    }

    /// Grows the focused pane by moving the nearest boundary the arrow can move.
    ///
    /// The arrow points at the boundary to push: with focus on the left of a
    /// horizontal split, Right grows it; with focus on the right, Left grows
    /// it. A direction with no boundary on that side of the pane is a no-op, so
    /// the arrow never shrinks a pane against an outer edge.
    pub fn resize(&mut self, target: PaneId, direction: Direction, delta: f32) -> bool {
        let Self::Split {
            axis,
            ratio,
            first,
            second,
        } = self
        else {
            return false;
        };

        let in_first = first.contains(target);
        let in_second = second.contains(target);
        if !in_first && !in_second {
            return false;
        }

        // Move the closest boundary first, so a nested split takes the change
        // before its parent does.
        if in_first && first.resize(target, direction, delta) {
            return true;
        }
        if in_second && second.resize(target, direction, delta) {
            return true;
        }

        let signal = match (axis, direction) {
            (Axis::Horizontal, Direction::Right) if in_first => Some(1.0),
            (Axis::Horizontal, Direction::Left) if in_second => Some(-1.0),
            (Axis::Vertical, Direction::Down) if in_first => Some(1.0),
            (Axis::Vertical, Direction::Up) if in_second => Some(-1.0),
            _ => None,
        };

        if let Some(signal) = signal {
            *ratio = (*ratio + delta * signal).clamp(MIN_RATIO, MAX_RATIO);
            return true;
        }

        false
    }

    /// Returns every pane and the fraction of the area it occupies.
    pub fn rects(&self) -> Vec<(PaneId, Fraction)> {
        let mut rects = Vec::new();
        self.collect_rects(Fraction::FULL, &mut rects);
        rects
    }

    fn collect_rects(&self, rect: Fraction, rects: &mut Vec<(PaneId, Fraction)>) {
        match self {
            Self::Leaf(id) => rects.push((*id, rect)),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) = divide(rect, *axis, *ratio);
                first.collect_rects(first_rect, rects);
                second.collect_rects(second_rect, rects);
            }
        }
    }
}

/// Splits `rect` into the parts its two children occupy.
fn divide(rect: Fraction, axis: Axis, ratio: f32) -> (Fraction, Fraction) {
    match axis {
        Axis::Horizontal => (
            Fraction {
                width: rect.width * ratio,
                ..rect
            },
            Fraction {
                x: rect.x + rect.width * ratio,
                width: rect.width * (1.0 - ratio),
                ..rect
            },
        ),
        Axis::Vertical => (
            Fraction {
                height: rect.height * ratio,
                ..rect
            },
            Fraction {
                y: rect.y + rect.height * ratio,
                height: rect.height * (1.0 - ratio),
                ..rect
            },
        ),
    }
}

/// Returns the pane nearest to `current` in `direction`.
///
/// The distance is between centres, so a pane directly beside the current one
/// wins over a diagonal one.
pub fn neighbour(rects: &[(PaneId, Fraction)], current: PaneId, direction: Direction) -> Option<PaneId> {
    let (_, current_rect) = rects.iter().find(|(id, _)| *id == current)?;
    let (current_x, current_y) = current_rect.center();

    rects
        .iter()
        .filter(|(id, _)| *id != current)
        .filter(|(_, rect)| {
            let (x, y) = rect.center();
            match direction {
                Direction::Left => x < current_x,
                Direction::Right => x > current_x,
                Direction::Up => y < current_y,
                Direction::Down => y > current_y,
            }
        })
        .min_by(|(_, a), (_, b)| {
            let (ax, ay) = a.center();
            let (bx, by) = b.center();
            let da = (ax - current_x).powi(2) + (ay - current_y).powi(2);
            let db = (bx - current_x).powi(2) + (by - current_y).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(id, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn assert_rect(actual: Fraction, expected: Fraction) {
        assert!(
            close(actual.x, expected.x)
                && close(actual.y, expected.y)
                && close(actual.width, expected.width)
                && close(actual.height, expected.height),
            "expected {expected:?}, got {actual:?}"
        );
    }

    /// A 2x2 grid: 0 and 2 on the left, 1 and 3 on the right, 0 and 1 on top.
    fn grid() -> Layout {
        let mut layout = Layout::leaf(PaneId(0));
        assert!(layout.split(PaneId(0), Axis::Horizontal, PaneId(1)));
        assert!(layout.split(PaneId(0), Axis::Vertical, PaneId(2)));
        assert!(layout.split(PaneId(1), Axis::Vertical, PaneId(3)));
        layout
    }

    #[test]
    fn a_leaf_fills_its_area() {
        let rects = Layout::leaf(PaneId(7)).rects();
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].0, PaneId(7));
        assert_rect(rects[0].1, Fraction::FULL);
    }

    #[test]
    fn a_split_divides_the_axis_it_names() {
        let mut layout = Layout::leaf(PaneId(0));
        assert!(layout.split(PaneId(0), Axis::Horizontal, PaneId(1)));

        let rects = layout.rects();
        assert_rect(
            rects[0].1,
            Fraction {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
        );
        assert_rect(
            rects[1].1,
            Fraction {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
        );

        let mut layout = Layout::leaf(PaneId(0));
        assert!(layout.split(PaneId(0), Axis::Vertical, PaneId(1)));
        let rects = layout.rects();
        assert_rect(
            rects[0].1,
            Fraction {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 0.5,
            },
        );
        assert_rect(
            rects[1].1,
            Fraction {
                x: 0.0,
                y: 0.5,
                width: 1.0,
                height: 0.5,
            },
        );
    }

    #[test]
    fn a_nested_split_stays_inside_its_parent() {
        let rects = grid().rects();
        assert_eq!(rects.len(), 4);

        assert_rect(
            rects[0].1,
            Fraction {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
        );
        assert_rect(
            rects[1].1,
            Fraction {
                x: 0.0,
                y: 0.5,
                width: 0.5,
                height: 0.5,
            },
        );
        assert_rect(
            rects[2].1,
            Fraction {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
        );
        assert_rect(
            rects[3].1,
            Fraction {
                x: 0.5,
                y: 0.5,
                width: 0.5,
                height: 0.5,
            },
        );
    }

    #[test]
    fn splitting_an_unknown_pane_is_a_no_op() {
        let mut layout = Layout::leaf(PaneId(0));
        assert!(!layout.split(PaneId(9), Axis::Horizontal, PaneId(1)));
        assert_eq!(layout.leaves(), vec![PaneId(0)]);
    }

    #[test]
    fn removing_a_pane_collapses_its_parent_to_the_sibling() {
        let mut layout = grid();
        assert!(layout.remove(PaneId(2)));
        // The left half is now pane 0, whole-height.
        let rects = layout.rects();
        assert_eq!(rects.len(), 3);
        assert_rect(
            rects[0].1,
            Fraction {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
        );

        // Removing a first child collapses to its sibling too.
        let mut layout = grid();
        assert!(layout.remove(PaneId(0)));
        let rects = layout.rects();
        assert_eq!(rects.len(), 3);
        assert_eq!(rects[0].0, PaneId(2));
        assert_rect(
            rects[0].1,
            Fraction {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
        );

        // Removing the root's only pane is refused.
        let mut single = Layout::leaf(PaneId(0));
        assert!(!single.remove(PaneId(0)));
        assert_eq!(single.leaves(), vec![PaneId(0)]);
    }

    #[test]
    fn resizing_moves_the_boundary_towards_the_direction() {
        let mut layout = Layout::leaf(PaneId(0));
        layout.split(PaneId(0), Axis::Horizontal, PaneId(1));

        // Pane 0 is on the left; Right grows it.
        assert!(layout.resize(PaneId(0), Direction::Right, 0.1));
        let rects = layout.rects();
        assert!(close(rects[0].1.width, 0.6));
        assert!(close(rects[1].1.width, 0.4));
        // The second pane starts where the first ends.
        assert!(close(rects[1].1.x, 0.6));

        // There is no pane to the left of 0, so Left does nothing.
        assert!(!layout.resize(PaneId(0), Direction::Left, 0.1));
        assert!(close(layout.rects()[0].1.width, 0.6));

        // Left with focus on the right pane grows it.
        assert!(layout.resize(PaneId(1), Direction::Left, 0.1));
        assert!(close(layout.rects()[0].1.width, 0.5));
    }

    #[test]
    fn resizing_reaches_the_nearest_split_on_that_axis() {
        // 0 is the left half; 1 is the right half's top, 2 its bottom.
        let mut layout = Layout::leaf(PaneId(0));
        layout.split(PaneId(0), Axis::Horizontal, PaneId(1));
        layout.split(PaneId(1), Axis::Vertical, PaneId(2));

        // Down grows the top-right pane, so the vertical split takes the
        // change and the outer split keeps its 50/50 width.
        let before = layout.rects();
        let top_before = before[1].1.height;
        assert!(layout.resize(PaneId(1), Direction::Down, 0.1));
        let after = layout.rects();
        assert!(after[1].1.height > top_before);
        assert!(after[2].1.height < before[2].1.height);
        assert!(close(after[0].1.width, 0.5));

        // The bottom pane has no boundary below it.
        assert!(!layout.resize(PaneId(2), Direction::Down, 0.1));
    }

    #[test]
    fn resizing_clamps_the_ratio() {
        let mut layout = Layout::leaf(PaneId(0));
        layout.split(PaneId(0), Axis::Horizontal, PaneId(1));

        assert!(layout.resize(PaneId(0), Direction::Right, 5.0));
        assert!(close(layout.rects()[0].1.width, MAX_RATIO));

        assert!(layout.resize(PaneId(1), Direction::Left, 5.0));
        assert!(close(layout.rects()[0].1.width, MIN_RATIO));
    }

    #[test]
    fn focus_walks_to_the_nearest_pane_in_the_direction() {
        let layout = grid();
        let rects = layout.rects();

        assert_eq!(neighbour(&rects, PaneId(0), Direction::Right), Some(PaneId(1)));
        assert_eq!(neighbour(&rects, PaneId(0), Direction::Down), Some(PaneId(2)));
        assert_eq!(neighbour(&rects, PaneId(3), Direction::Left), Some(PaneId(2)));
        assert_eq!(neighbour(&rects, PaneId(3), Direction::Up), Some(PaneId(1)));
        assert_eq!(neighbour(&rects, PaneId(1), Direction::Left), Some(PaneId(0)));

        // Nothing lies beyond the edges.
        assert_eq!(neighbour(&rects, PaneId(0), Direction::Left), None);
        assert_eq!(neighbour(&rects, PaneId(0), Direction::Up), None);
    }
}
