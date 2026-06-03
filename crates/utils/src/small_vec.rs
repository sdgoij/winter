//! Stack-optimized small vector.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::small_vector`.

use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::ptr;

/// A small vector that stores up to `N` elements on the stack.
///
/// Ported from `til::small_vector<T, N>` in the Windows Terminal C++ codebase.
/// For `N` elements, stores them inline. Beyond that, falls back to heap allocation.
pub struct SmallVec<T, const N: usize> {
    // Inlined storage for the first N elements
    inline: [MaybeUninit<T>; N],
    // Number of initialized elements
    len: usize,
    // Whether we're currently using the heap
    use_heap: bool,
    // Heap storage (only valid when use_heap is true)
    heap: Option<Box<[MaybeUninit<T>]>>,
}

impl<T, const N: usize> SmallVec<T, N> {
    /// Creates a new empty small vector.
    #[inline]
    pub const fn new() -> Self {
        SmallVec {
            inline: unsafe { MaybeUninit::uninit().assume_init() },
            len: 0,
            use_heap: false,
            heap: None,
        }
    }

    /// Creates a new empty small vector with pre-allocated heap capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        if capacity <= N {
            SmallVec::new()
        } else {
            SmallVec {
                inline: unsafe { MaybeUninit::uninit().assume_init() },
                len: 0,
                use_heap: true,
                heap: Some(
                    std::iter::repeat_with(|| MaybeUninit::uninit())
                        .take(capacity)
                        .collect(),
                ),
            }
        }
    }

    /// Creates a small vector from a `Vec<T>`.
    pub fn from_vec(v: Vec<T>) -> Self {
        if v.len() <= N {
            let mut result = SmallVec::new();
            unsafe {
                let inline_ptr = result.inline.as_mut_ptr();
                for item in v {
                    ptr::write(inline_ptr.add(result.len) as *mut T, item);
                    result.len += 1;
                }
            }
            result
        } else {
            let result = SmallVec {
                inline: unsafe { MaybeUninit::uninit().assume_init() },
                len: v.len(),
                use_heap: true,
                heap: Some(
                    v.into_iter()
                        .map(|x| MaybeUninit::new(x))
                        .collect::<Box<[MaybeUninit<T>]>>(),
                ),
            };
            result
        }
    }

    /// Returns the number of elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the vector is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        if self.use_heap {
            self.heap.as_ref().map_or(N, |h| h.len())
        } else {
            N
        }
    }

    /// Returns whether elements are stored inline (on the stack).
    #[inline]
    pub fn is_inline(&self) -> bool {
        !self.use_heap
    }

    /// Returns a reference to the element at the given index.
    #[inline]
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        unsafe {
            if self.use_heap {
                Some(
                    self.heap
                        .as_ref()
                        .unwrap_unchecked()
                        .get(index)?
                        .assume_init_ref(),
                )
            } else {
                Some(self.inline.get(index)?.assume_init_ref())
            }
        }
    }

    /// Returns a mutable reference to the element at the given index.
    #[inline]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len {
            return None;
        }
        unsafe {
            if self.use_heap {
                Some(
                    self.heap
                        .as_mut()
                        .unwrap_unchecked()
                        .get_mut(index)?
                        .assume_init_mut(),
                )
            } else {
                Some(self.inline.get_mut(index)?.assume_init_mut())
            }
        }
    }

    /// Pushes an element onto the vector.
    pub fn push(&mut self, value: T) {
        if self.len < self.capacity() {
            self.push_inline(value);
        } else {
            self.grow();
            self.push_inline(value);
        }
    }

    /// Pops the last element from the vector.
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        unsafe {
            if self.use_heap {
                Some(
                    self.heap
                        .as_mut()
                        .unwrap_unchecked()
                        .get_mut(self.len)?
                        .assume_init_read(),
                )
            } else {
                Some(self.inline.get_mut(self.len)?.assume_init_read())
            }
        }
    }

    /// Inserts an element at the given position, shifting subsequent elements.
    pub fn insert(&mut self, index: usize, value: T) {
        if index == self.len {
            self.push(value);
            return;
        }
        if self.len < self.capacity() {
            // Shift elements within current storage
            unsafe {
                if self.use_heap {
                    let heap = self.heap.as_mut().unwrap_unchecked();
                    ptr::copy(
                        heap.get_unchecked(index).as_ptr(),
                        heap.get_unchecked_mut(index + 1).as_mut_ptr(),
                        self.len - index,
                    );
                    ptr::write(heap.get_unchecked_mut(index).as_mut_ptr(), value);
                } else {
                    ptr::copy(
                        self.inline.get_unchecked(index).as_ptr(),
                        self.inline.get_unchecked_mut(index + 1).as_mut_ptr(),
                        self.len - index,
                    );
                    ptr::write(self.inline.get_unchecked_mut(index).as_mut_ptr(), value);
                }
            }
            self.len += 1;
        } else {
            self.grow();
            unsafe {
                if self.use_heap {
                    let heap = self.heap.as_mut().unwrap_unchecked();
                    ptr::copy(
                        heap.get_unchecked(index).as_ptr(),
                        heap.get_unchecked_mut(index + 1).as_mut_ptr(),
                        self.len - index,
                    );
                    ptr::write(heap.get_unchecked_mut(index).as_mut_ptr(), value);
                } else {
                    ptr::copy(
                        self.inline.get_unchecked(index).as_ptr(),
                        self.inline.get_unchecked_mut(index + 1).as_mut_ptr(),
                        self.len - index,
                    );
                    ptr::write(self.inline.get_unchecked_mut(index).as_mut_ptr(), value);
                }
            }
            self.len += 1;
        }
    }

    /// Removes and returns the element at the given position.
    pub fn remove(&mut self, index: usize) -> Option<T> {
        if index >= self.len {
            return None;
        }
        let value = unsafe {
            if self.use_heap {
                self.heap
                    .as_mut()
                    .unwrap_unchecked()
                    .get_mut(index)?
                    .assume_init_read()
            } else {
                self.inline.get_mut(index)?.assume_init_read()
            }
        };
        unsafe {
            if self.use_heap {
                let heap = self.heap.as_mut().unwrap_unchecked();
                ptr::copy(
                    heap.get_unchecked(index + 1).as_ptr(),
                    heap.get_unchecked_mut(index).as_mut_ptr(),
                    self.len - index - 1,
                );
            } else {
                ptr::copy(
                    self.inline.get_unchecked(index + 1).as_ptr(),
                    self.inline.get_unchecked_mut(index).as_mut_ptr(),
                    self.len - index - 1,
                );
            }
        }
        self.len -= 1;
        Some(value)
    }

    /// Clears all elements from the vector.
    pub fn clear(&mut self) {
        while self.pop().is_some() {}
    }

    /// Shrinks the capacity to match the length.
    pub fn shrink_to_fit(&mut self) {
        if self.len == 0 && self.use_heap {
            self.heap = None;
            self.use_heap = false;
        }
    }

    fn push_inline(&mut self, value: T) {
        let idx = self.len;
        self.len += 1;
        unsafe {
            if self.use_heap {
                self.heap
                    .as_mut()
                    .unwrap_unchecked()
                    .get_unchecked_mut(idx)
                    .write(value);
            } else {
                self.inline.get_unchecked_mut(idx).write(value);
            }
        }
    }

    fn grow(&mut self) {
        let new_cap = if self.use_heap {
            self.heap.as_ref().map_or(N, |h| h.len()) * 2
        } else {
            N * 2
        };

        let mut new_heap: Box<[MaybeUninit<T>]> = std::iter::repeat_with(|| MaybeUninit::uninit())
            .take(new_cap)
            .collect();

        // Copy existing elements
        unsafe {
            for i in 0..self.len {
                if self.use_heap {
                    let src = self.heap.as_ref().unwrap_unchecked().get_unchecked(i);
                    ptr::write(
                        new_heap.get_unchecked_mut(i).as_mut_ptr(),
                        src.assume_init_read(),
                    );
                } else {
                    let src = self.inline.get_unchecked(i);
                    ptr::write(
                        new_heap.get_unchecked_mut(i).as_mut_ptr(),
                        src.assume_init_read(),
                    );
                }
            }
        }

        self.use_heap = true;
        self.heap = Some(new_heap);
    }
}

impl<T, const N: usize> Default for SmallVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Deref for SmallVec<T, N> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T, const N: usize> DerefMut for SmallVec<T, N> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}

impl<T, const N: usize> AsRef<[T]> for SmallVec<T, N> {
    fn as_ref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T, const N: usize> SmallVec<T, N> {
    /// Returns a slice of the elements.
    pub fn as_slice(&self) -> &[T] {
        unsafe {
            if self.use_heap {
                let ptr = self.heap.as_ref().unwrap_unchecked().as_ptr() as *const T;
                std::slice::from_raw_parts(ptr, self.len)
            } else {
                let ptr = self.inline.as_ptr() as *const T;
                std::slice::from_raw_parts(ptr, self.len)
            }
        }
    }

    /// Returns a mutable slice of the elements.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        unsafe {
            if self.use_heap {
                let ptr = self.heap.as_mut().unwrap_unchecked().as_mut_ptr() as *mut T;
                std::slice::from_raw_parts_mut(ptr, self.len)
            } else {
                let ptr = self.inline.as_mut_ptr() as *mut T;
                std::slice::from_raw_parts_mut(ptr, self.len)
            }
        }
    }
}

impl<T: std::fmt::Debug, const N: usize> std::fmt::Debug for SmallVec<T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.as_slice().iter()).finish()
    }
}

impl<T: PartialEq, const N: usize> PartialEq for SmallVec<T, N> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: Eq, const N: usize> Eq for SmallVec<T, N> {}

/// Iterator over references to elements.
pub struct Iter<'a, T, const N: usize> {
    inner: std::slice::Iter<'a, T>,
    _marker: std::marker::PhantomData<&'a T>,
}

impl<'a, T, const N: usize> Iterator for Iter<'a, T, N> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<'a, T, const N: usize> ExactSizeIterator for Iter<'a, T, N> {}

/// Iterator over mutable references to elements.
pub struct IterMut<'a, T, const N: usize> {
    inner: std::slice::IterMut<'a, T>,
}

impl<'a, T, const N: usize> Iterator for IterMut<'a, T, N> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<'a, T, const N: usize> ExactSizeIterator for IterMut<'a, T, N> {}

/// Iterator that consumes the small vector.
pub struct IntoIter<T, const N: usize> {
    vec: SmallVec<T, N>,
    index: usize,
}

impl<T, const N: usize> Iterator for IntoIter<T, N> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.vec.len {
            let idx = self.index;
            self.index += 1;
            self.vec.remove(idx)
        } else {
            None
        }
    }
}

impl<T, const N: usize> std::iter::FromIterator<T> for SmallVec<T, N> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut vec = SmallVec::new();
        for item in iter {
            vec.push(item);
        }
        vec
    }
}

impl<T, const N: usize> Extend<T> for SmallVec<T, N> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for item in iter {
            self.push(item);
        }
    }
}

impl<T, const N: usize> std::ops::Index<usize> for SmallVec<T, N> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("index out of bounds")
    }
}

impl<T, const N: usize> std::ops::IndexMut<usize> for SmallVec<T, N> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index).expect("index out of bounds")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_and_empty() {
        let sv: SmallVec<i32, 4> = SmallVec::new();
        assert_eq!(sv.len(), 0);
        assert!(sv.is_empty());
    }

    #[test]
    fn test_push_pop() {
        let mut sv: SmallVec<i32, 4> = SmallVec::new();
        sv.push(1);
        sv.push(2);
        sv.push(3);
        assert_eq!(sv.len(), 3);
        assert_eq!(*sv.get(0).unwrap(), 1);
        assert_eq!(*sv.get(1).unwrap(), 2);
        assert_eq!(*sv.get(2).unwrap(), 3);

        assert_eq!(sv.pop(), Some(3));
        assert_eq!(sv.pop(), Some(2));
        assert_eq!(sv.pop(), Some(1));
        assert_eq!(sv.pop(), None);
    }

    #[test]
    fn test_push_beyond_inline_capacity() {
        let mut sv: SmallVec<i32, 2> = SmallVec::new();
        sv.push(1);
        sv.push(2);
        sv.push(3);
        assert!(!sv.is_inline());
        assert_eq!(sv.len(), 3);
        assert_eq!(*sv.get(0).unwrap(), 1);
        assert_eq!(*sv.get(1).unwrap(), 2);
        assert_eq!(*sv.get(2).unwrap(), 3);
    }

    #[test]
    fn test_insert_remove() {
        let mut sv: SmallVec<i32, 4> = SmallVec::new();
        sv.push(1);
        sv.push(2);
        sv.push(3);

        sv.insert(1, 99);
        assert_eq!(*sv.get(1).unwrap(), 99);
        assert_eq!(*sv.get(2).unwrap(), 2);
        assert_eq!(*sv.get(3).unwrap(), 3);

        let removed = sv.remove(1);
        assert_eq!(removed, Some(99));
        assert_eq!(sv.len(), 3);
        assert_eq!(*sv.get(1).unwrap(), 2);
    }

    #[test]
    fn test_clear() {
        let mut sv: SmallVec<i32, 4> = SmallVec::new();
        sv.push(1);
        sv.push(2);
        sv.clear();
        assert_eq!(sv.len(), 0);
    }

    #[test]
    fn test_deref() {
        let mut sv: SmallVec<i32, 4> = SmallVec::new();
        sv.push(1);
        sv.push(2);
        sv.push(3);

        assert_eq!(sv.len(), 3);
        assert_eq!(sv[0], 1);
        sv[0] = 10;
        assert_eq!(sv[0], 10);
    }

    #[test]
    fn test_iter() {
        let mut sv: SmallVec<i32, 4> = SmallVec::new();
        sv.push(1);
        sv.push(2);
        sv.push(3);

        let collected: Vec<&i32> = sv.iter().collect();
        assert_eq!(collected, vec![&1, &2, &3]);
    }

    #[test]
    fn test_from_vec() {
        let v = vec![1, 2, 3];
        let sv: SmallVec<i32, 4> = SmallVec::from_vec(v);
        assert_eq!(sv.as_slice(), &[1, 2, 3]);
        assert!(sv.is_inline());

        let v2 = vec![1, 2, 3, 4, 5];
        let sv2: SmallVec<i32, 2> = SmallVec::from_vec(v2);
        assert_eq!(sv2.as_slice(), &[1, 2, 3, 4, 5]);
        assert!(!sv2.is_inline());
    }

    #[test]
    fn test_debug() {
        let mut sv: SmallVec<i32, 4> = SmallVec::new();
        sv.push(1);
        sv.push(2);
        let debug_str = format!("{:?}", sv);
        assert!(debug_str.contains("1"));
        assert!(debug_str.contains("2"));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_push_pop_roundtrip(items in prop::collection::vec(any::<i32>(), 0..10)) {
            let mut sv: SmallVec<i32, 4> = SmallVec::new();
            for item in &items {
                sv.push(*item);
            }
            let mut popped = Vec::new();
            while let Some(v) = sv.pop() {
                popped.push(v);
            }
            popped.reverse();
            assert_eq!(popped, items);
        }

        #[test]
        fn test_as_slice_matches_iter(sv in prop::collection::vec(any::<i32>(), 0..10)) {
            let mut small_vec: SmallVec<i32, 4> = SmallVec::new();
            for item in &sv {
                small_vec.push(*item);
            }
            let slice = small_vec.as_slice();
            let iter_vec: Vec<&i32> = small_vec.iter().collect();
            assert_eq!(slice.iter().collect::<Vec<&i32>>(), iter_vec);
        }
    }
}
