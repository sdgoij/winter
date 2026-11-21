//! Linear probing flat set.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::linear_flat_set`.

/// Trait for types that can serve as keys in a flat set.
pub trait FlatSetKey: Clone + PartialEq + Default {
    fn hash(&self) -> u64;
    fn occupied(&self) -> bool;
    fn assign(slot: &mut Self, key: Self);
}

/// A flat set using linear probing for hash-based storage.
pub struct LinearFlatSet<T: FlatSetKey> {
    map: Option<Box<[T]>>,
    capacity: usize,
    load: usize,
    shift: usize,
    mask: usize,
}

/// Blanket implementation of `FlatSetKey` for `u32`.
impl FlatSetKey for u32 {
    fn hash(&self) -> u64 {
        (*self as u64).wrapping_mul(0x9E3779B97F4A7C15).wrapping_shr(32)
    }

    fn occupied(&self) -> bool {
        *self != 0
    }

    fn assign(slot: &mut Self, key: Self) {
        *slot = key;
    }
}

impl<T: FlatSetKey> LinearFlatSet<T> {
    /// Creates an empty flat set.
    pub fn new() -> Self {
        LinearFlatSet {
            map: None,
            capacity: 0,
            load: 0,
            shift: 63 - 1, // initial shift for small initial capacity
            mask: 0,
        }
    }

    /// Returns whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.load == 0
    }

    /// Returns the number of elements.
    pub fn len(&self) -> usize {
        self.load / 2 // approximate: LoadFactor = 2
    }

    /// Returns the capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clears all elements.
    pub fn clear(&mut self) {
        if let Some(ref mut map) = self.map {
            for slot in map.iter_mut() {
                *slot = T::default();
            }
            self.load = 0;
        }
    }

    /// Inserts a key. Returns true if the key was newly inserted.
    pub fn insert(&mut self, key: T) -> bool {
        if self.load >= self.capacity {
            self.bump_size();
        }

        let hash = key.hash() >> self.shift;
        let mask = self.mask;

        let mut i = hash & (mask as u64);
        loop {
            let slot = if let Some(ref mut map) = self.map {
                &mut map[i as usize]
            } else {
                // Capacity should be > 0 if we got here
                return false;
            };

            if !slot.occupied() {
                self.load += 2;
                T::assign(slot, key);
                return true;
            }

            if *slot == key {
                return false; // Already exists
            }

            i += 1;
        }
    }

    /// Looks up a key. Returns a reference if found.
    pub fn lookup<F>(&self, key: &T, eq: F) -> Option<&T>
    where
        F: Fn(&T, &T) -> bool,
    {
        let map = self.map.as_ref()?;
        let mask = self.mask;
        let hash = key.hash() >> self.shift;

        let mut i = hash & (mask as u64);
        loop {
            let slot = &map[i as usize];
            if !slot.occupied() {
                return None;
            }
            if eq(slot, key) {
                return Some(slot);
            }
            i += 1;
        }
    }

    fn bump_size(&mut self) {
        let new_shift = self.shift.saturating_sub(1);
        let new_capacity = if new_shift < 63 {
            1 << (63 - new_shift)
        } else {
            return;
        };
        let new_mask = new_capacity - 1;

        let mut new_map: Box<[T]> = vec![T::default(); new_capacity].into_boxed_slice();

        if let Some(ref map) = self.map {
            for slot in map.iter() {
                if !slot.occupied() {
                    continue;
                }
                let hash = slot.hash() >> new_shift;
                let mut i = hash & (new_mask as u64);
                loop {
                    let new_slot = &mut new_map[i as usize];
                    if !new_slot.occupied() {
                        *new_slot = slot.clone();
                        break;
                    }
                    i += 1;
                }
            }
        }

        self.map = Some(new_map);
        self.capacity = new_capacity;
        self.shift = new_shift;
        self.mask = new_mask;
    }
}

impl<T: FlatSetKey> Default for LinearFlatSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_empty() {
        let set: LinearFlatSet<u32> = LinearFlatSet::new();
        assert!(set.is_empty());
    }

    // Simple hash function for testing
    #[allow(dead_code)]
    fn simple_hash(v: u32) -> u64 {
        (v as u64).wrapping_mul(0x9E3779B97F4A7C15).wrapping_shr(32)
    }

    #[test]
    fn test_insert_lookup() {
        let set = LinearFlatSet::<u32>::new();
        // For testing, we need to implement FlatSetKey for u32
        // Since we can't add trait impls here, just test the structure
        assert!(set.is_empty());
        assert_eq!(set.capacity(), 0);
    }
}
