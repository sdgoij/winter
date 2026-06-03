//! Cross-platform mutex.

use std::sync::{Mutex as StdMutex, MutexGuard};
use std::time::Duration;

/// A cross-platform mutex.
#[derive(Debug)]
pub struct Mutex<T: Send> {
    inner: StdMutex<T>,
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T: Send> Mutex<T> {
    /// Creates a new mutex wrapping the given value.
    pub const fn new(value: T) -> Self {
        Mutex {
            inner: StdMutex::new(value),
        }
    }

    /// Locks the mutex, blocking until available.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        self.inner.lock().expect("Mutex poisoned")
    }

    /// Tries to lock the mutex without blocking.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.inner.try_lock().ok()
    }

    /// Locks with a timeout.
    pub fn lock_timeout(&self, timeout: Duration) -> Option<MutexGuard<'_, T>> {
        // std::sync::Mutex doesn't have a timeout, so we use try_lock in a loop
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if let Some(guard) = self.try_lock() {
                return Some(guard);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        None
    }

    /// Returns a mutable reference to the contained value (requires `&mut self`).
    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut().expect("Mutex poisoned")
    }
}

impl<T: Send> Default for Mutex<T>
where
    T: Default,
{
    fn default() -> Self {
        Mutex::new(T::default())
    }
}

impl<T: Send + std::fmt::Display> std::fmt::Display for Mutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Mutex({})", self.inner.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_mutex_basic() {
        let m = Mutex::new(42);
        assert_eq!(*m.lock(), 42);
    }

    #[test]
    fn test_mutex_lock_unlock() {
        let m = Mutex::new(0);
        {
            let mut guard = m.lock();
            *guard += 1;
        }
        assert_eq!(*m.lock(), 1);
    }

    #[test]
    fn test_mutex_try_lock() {
        let m = Mutex::new(42);
        {
            let guard = m.try_lock();
            assert!(guard.is_some());
            drop(guard);
        }
        assert!(m.try_lock().is_some());
    }

    #[test]
    fn test_mutex_concurrent() {
        let m = Arc::new(Mutex::new(0i32));
        let mut handles = vec![];

        for _ in 0..10 {
            let m = Arc::clone(&m);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    *m.lock() += 1;
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(*m.lock(), 1000);
    }

    #[test]
    fn test_mutex_get_mut() {
        let mut m = Mutex::new(42);
        *m.get_mut() = 100;
        assert_eq!(*m.lock(), 100);
    }
}
