//! One-shot synchronization barrier (countdown latch).
//!
//! Ported from Microsoft's Windows Terminal C++ `til::latch`.

use std::sync::{Condvar, Mutex as StdMutex};

/// A countdown latch for thread synchronization.
///
/// Threads can `count_down()` the latch, and other threads can `wait()`
/// for it to reach zero.
#[derive(Debug)]
pub struct Latch {
    counter: StdMutex<usize>,
    condvar: Condvar,
}

impl Latch {
    /// Creates a new latch with the given count.
    pub const fn new(count: usize) -> Self {
        Latch {
            counter: StdMutex::new(count),
            condvar: Condvar::new(),
        }
    }

    /// Decrements the count by n.
    pub fn count_down(&self, n: usize) {
        let mut counter = self.counter.lock().unwrap();
        *counter = counter.saturating_sub(n);
        if *counter == 0 {
            self.condvar.notify_all();
        }
    }

    /// Blocks until the count reaches zero.
    pub fn wait(&self) {
        let mut counter = self.counter.lock().unwrap();
        while *counter > 0 {
            counter = self.condvar.wait(counter).unwrap();
        }
    }

    /// Non-blocking check. Returns true if the count has reached zero.
    pub fn try_wait(&self) -> bool {
        *self.counter.lock().unwrap() == 0
    }

    /// Atomically decrements by n and waits.
    pub fn arrive_and_wait(&self, n: usize) {
        let mut counter = self.counter.lock().unwrap();
        *counter = counter.saturating_sub(n);
        if *counter == 0 {
            self.condvar.notify_all();
        }
        while *counter > 0 {
            counter = self.condvar.wait(counter).unwrap();
        }
    }

    /// Returns the remaining count.
    pub fn count(&self) -> usize {
        *self.counter.lock().unwrap()
    }
}

impl Default for Latch {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_latch_basic() {
        let latch = Latch::new(1);
        latch.count_down(1);
        latch.wait();
    }

    #[test]
    fn test_latch_wait_in_another_thread() {
        let latch = Arc::new(Latch::new(1));
        let latch_clone = Arc::clone(&latch);

        let handle = thread::spawn(move || {
            latch_clone.wait();
        });

        thread::sleep(Duration::from_millis(10));
        latch.count_down(1);
        handle.join().unwrap();
    }

    #[test]
    fn test_latch_multiple_count_downs() {
        let latch = Arc::new(Latch::new(3));
        let latch_clone = Arc::clone(&latch);

        let handle = thread::spawn(move || {
            latch_clone.wait();
        });

        latch.count_down(1);
        latch.count_down(1);
        thread::sleep(Duration::from_millis(10));
        latch.count_down(1);
        handle.join().unwrap();
    }

    #[test]
    fn test_latch_try_wait() {
        let latch = Latch::new(1);
        assert!(!latch.try_wait());
        latch.count_down(1);
        assert!(latch.try_wait());
    }

    #[test]
    fn test_latch_arrive_and_wait() {
        let latch = Arc::new(Latch::new(2));
        let latch_clone = Arc::clone(&latch);

        let handle1 = thread::spawn(move || {
            latch_clone.arrive_and_wait(1);
        });

        let handle2 = thread::spawn(move || {
            latch.arrive_and_wait(1);
        });

        handle1.join().unwrap();
        handle2.join().unwrap();
    }

    #[test]
    fn test_latch_multiple_waiters() {
        let latch = Arc::new(Latch::new(1));
        let mut handles = vec![];

        for _ in 0..5 {
            let latch_clone = Arc::clone(&latch);
            handles.push(thread::spawn(move || {
                latch_clone.wait();
            }));
        }

        thread::sleep(Duration::from_millis(10));
        latch.count_down(1);

        for h in handles {
            h.join().unwrap();
        }
    }
}
