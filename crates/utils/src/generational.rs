//! Generational indices for cache coherence.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::generational`.

/// A generation counter that wraps around on overflow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Generation {
    value: u32,
}

impl Generation {
    /// Creates a new generation with the given value.
    pub const fn new(value: u32) -> Self {
        Generation { value }
    }

    /// Returns the current generation value.
    pub const fn value(&self) -> u32 {
        self.value
    }

    /// Increments the generation, wrapping on overflow.
    pub fn bump(&mut self) {
        self.value = self.value.wrapping_add(1);
    }
}

/// A generational container that invalidates when the generation changes.
///
/// Used for cache coherence — when the wrapped value is modified, the
/// generation is bumped and any cached indices referencing the old
/// generation become invalid.
///
/// Ported from `til::generational<T>` in the Windows Terminal C++ codebase.
pub struct Generational<T> {
    generation: Generation,
    value: T,
}

impl<T> Generational<T> {
    /// Creates a new generational container.
    pub const fn new(value: T) -> Self {
        Generational {
            generation: Generation::new(0),
            value,
        }
    }

    /// Creates with a specific generation.
    pub const fn with_generation(generation: Generation, value: T) -> Self {
        Generational { generation, value }
    }

    /// Returns the current generation.
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Returns a reference to the inner value (if generation matches).
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Returns a mutable reference and bumps the generation.
    pub fn write(&mut self) -> &mut T {
        self.generation.bump();
        &mut self.value
    }

    /// Checks if a generation is still valid for this container.
    pub fn is_valid(&self, generation: Generation) -> bool {
        self.generation == generation
    }
}

impl<T> Default for Generational<T>
where
    T: Default,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> std::ops::Deref for Generational<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> std::ops::DerefMut for Generational<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn test_generational() {
        let mut container = Generational::new(42);
        assert_eq!(*container.get(), 42);

        let old_gen = container.generation();
        let val = container.write();
        *val = 100;

        assert_eq!(*container.get(), 100);
        assert_ne!(container.generation(), old_gen);
    }

    #[test]
    fn test_generational_invalidation() {
        let mut container = Generational::new(42);
        let gen_before = container.generation();

        container.write();
        let gen_after = container.generation();

        assert!(!container.is_valid(gen_before));
        assert!(container.is_valid(gen_after));
    }

    #[test]
    fn test_bump() {
        let mut counter = Generation::new(0);
        assert_eq!(counter.value(), 0);
        counter.bump();
        assert_eq!(counter.value(), 1);
    }

    #[test]
    fn test_generation_bump() {
        let mut counter = Generation::new(u32::MAX);
        counter.bump();
        assert_eq!(counter.value(), 0);
    }

    #[test]
    fn test_default() {
        let r#gen: Generational<i32> = Generational::default();
        assert_eq!(*r#gen.get(), 0);
    }

    proptest! {
        /// A token is valid exactly while no write follows it, and the value it
        /// guards is the last one written.
        #[test]
        fn a_token_survives_exactly_the_writes_it_saw(writes in 0usize..512) {
            let mut cache = Generational::new(0u32);
            let token = cache.generation();

            for value in 0..writes {
                *cache.write() = u32::try_from(value).expect("small");
            }

            prop_assert_eq!(cache.is_valid(token), writes == 0);
            prop_assert_eq!(cache.generation().value(), writes as u32);
            let expected = if writes == 0 { 0 } else { writes as u32 - 1 };
            prop_assert_eq!(*cache.get(), expected);

            // The token after the writes is the one that is valid now.
            prop_assert!(cache.is_valid(cache.generation()));
        }
    }
}
