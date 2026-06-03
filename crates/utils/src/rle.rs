//! Run-length encoding.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::rle`.

/// A single run in a run-length encoded sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RleRun<V> {
    pub value: V,
    pub length: usize,
}

impl<V> RleRun<V> {
    /// Creates a new run.
    pub const fn new(value: V, length: usize) -> Self {
        RleRun { value, length }
    }
}

/// A basic run-length encoded container.
///
/// Stores runs of identical consecutive values compactly.
/// Useful for compressing attribute changes in terminal buffers.
pub struct BasicRle<V: PartialEq + Clone> {
    runs: Vec<RleRun<V>>,
    total_length: usize,
}

impl<V: PartialEq + Clone> BasicRle<V> {
    /// Creates a new empty RLE container.
    pub fn new() -> Self {
        BasicRle {
            runs: Vec::new(),
            total_length: 0,
        }
    }

    /// Creates from an iterator of runs.
    pub fn from_runs(runs: Vec<RleRun<V>>) -> Self {
        let total_length: usize = runs.iter().map(|r| r.length).sum();
        BasicRle { runs, total_length }
    }

    /// Creates from a sequence of values, encoding consecutive runs.
    pub fn from_values(values: &[V]) -> Self
    where
        V: PartialEq + Clone,
    {
        let mut runs: Vec<RleRun<V>> = Vec::new();
        let mut total_length = 0;

        for value in values {
            if let Some(last) = runs.last_mut() {
                if last.value == *value {
                    last.length += 1;
                } else {
                    runs.push(RleRun::new(value.clone(), 1));
                }
            } else {
                runs.push(RleRun::new(value.clone(), 1));
            }
            total_length += 1;
        }

        BasicRle { runs, total_length }
    }

    /// Returns the number of runs.
    pub fn run_count(&self) -> usize {
        self.runs.len()
    }

    /// Returns the total length (sum of all run lengths).
    pub fn total_length(&self) -> usize {
        self.total_length
    }

    /// Returns whether the RLE is empty.
    pub fn is_empty(&self) -> bool {
        self.total_length == 0
    }

    /// Gets the value at a given position.
    pub fn at(&self, pos: usize) -> Option<&V> {
        let mut remaining = pos;
        for run in &self.runs {
            if remaining < run.length {
                return Some(&run.value);
            }
            remaining -= run.length;
        }
        None
    }

    /// Returns a reference to all runs.
    pub fn runs(&self) -> &[RleRun<V>] {
        &self.runs
    }

    /// Returns an iterator over the runs.
    pub fn iter_runs(&self) -> impl Iterator<Item = &RleRun<V>> {
        self.runs.iter()
    }

    /// Expands to a Vec of all values.
    pub fn expand(&self) -> Vec<V> {
        let mut result = Vec::with_capacity(self.total_length);
        for run in &self.runs {
            for _ in 0..run.length {
                result.push(run.value.clone());
            }
        }
        result
    }

    /// Compresses consecutive identical runs.
    pub fn compact(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        self.runs
            .retain_mut(|run| if run.length > 0 { true } else { false });

        // Merge consecutive runs with same value
        let mut i = 0;
        while i + 1 < self.runs.len() {
            if self.runs[i].value == self.runs[i + 1].value {
                self.runs[i].length += self.runs[i + 1].length;
                self.runs.remove(i + 1);
            } else {
                i += 1;
            }
        }

        // Recalculate total length
        self.total_length = self.runs.iter().map(|r| r.length).sum();
    }
}

impl<V: PartialEq + Clone> Default for BasicRle<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_empty() {
        let rle: BasicRle<i32> = BasicRle::new();
        assert!(rle.is_empty());
        assert_eq!(rle.total_length(), 0);
        assert_eq!(rle.run_count(), 0);
    }

    #[test]
    fn test_from_values_compression() {
        let values = vec![1, 1, 1, 2, 2, 3];
        let rle = BasicRle::from_values(&values);
        assert_eq!(rle.run_count(), 3);
        assert_eq!(rle.total_length(), 6);

        let expanded = rle.expand();
        assert_eq!(expanded, values);
    }

    #[test]
    fn test_at() {
        let values = vec![1, 1, 2, 2, 2, 3];
        let rle = BasicRle::from_values(&values);
        assert_eq!(rle.at(0), Some(&1));
        assert_eq!(rle.at(2), Some(&2));
        assert_eq!(rle.at(5), Some(&3));
        assert_eq!(rle.at(6), None);
    }

    #[test]
    fn test_compact() {
        let mut rle = BasicRle::from_runs(vec![
            RleRun::new(1, 3),
            RleRun::new(1, 2),
            RleRun::new(2, 4),
        ]);
        assert_eq!(rle.run_count(), 3);
        rle.compact();
        assert_eq!(rle.run_count(), 2);
        assert_eq!(rle.total_length(), 9);
    }
}
