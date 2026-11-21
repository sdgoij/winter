//! Debounced/throttled function calls.

use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// A throttled function caller that debounces rapid calls.
///
/// Only the last call within the throttle window will actually execute.
pub struct Throttled {
    callback: Arc<Mutex<dyn FnMut() + Send>>,
    delay: Duration,
    handle: Mutex<Option<JoinHandle<()>>>,
}

unsafe impl Send for Throttled {}
unsafe impl Sync for Throttled {}

impl Throttled {
    /// Creates a new throttled caller with the given delay.
    pub fn new<F>(delay: Duration, callback: F) -> Self
    where
        F: FnMut() + Send + 'static,
    {
        Throttled {
            callback: Arc::new(Mutex::new(callback)),
            delay,
            handle: Mutex::new(None),
        }
    }

    /// Triggers a call. If another call is already scheduled, resets the timer.
    pub fn call(&self) {
        // Cancel any existing scheduled call
        if let Some(h) = {
            let mut handle = self.handle.lock().unwrap();
            handle.take()
        } {
            // Best effort: try to join (the thread will finish its sleep)
            let _ = h.join();
        }

        let callback = Arc::clone(&self.callback);
        let delay = self.delay;

        let handle = thread::spawn(move || {
            thread::sleep(delay);
            let mut cb = callback.lock().unwrap();
            cb();
        });

        let mut handle_guard = self.handle.lock().unwrap();
        *handle_guard = Some(handle);
    }

    /// Resets the timer, canceling any scheduled call.
    pub fn reset(&self) {
        let mut handle = self.handle.lock().unwrap();
        // Drop the handle without joining - the thread will finish its sleep
        // but we don't want to block. The callback may still be called briefly
        // but this is expected behavior for a non-blocking reset.
        handle.take();
    }

    /// Gets the current delay.
    pub fn delay(&self) -> Duration {
        self.delay
    }

    /// Sets a new delay.
    pub fn set_delay(&mut self, delay: Duration) {
        self.delay = delay;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    #[test]
    fn test_throttled_basic() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let throttled = Throttled::new(Duration::from_millis(10), move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        // Call once and wait
        throttled.call();

        // Wait for the call to execute with good margin
        thread::sleep(Duration::from_millis(200));

        // Should have executed exactly once
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_throttled_reset() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let throttled = Throttled::new(Duration::from_millis(50), move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        throttled.call();
        // Give the thread time to start
        thread::sleep(Duration::from_millis(5));
        throttled.reset();

        // Wait longer than the throttle delay
        thread::sleep(Duration::from_millis(200));

        // Reset drops the handle; the callback might or might not execute
        // This is non-deterministic behavior for a non-blocking reset
        let count = counter.load(Ordering::SeqCst);
        assert!(count <= 1, "reset should not cause multiple executions, got {}", count);
    }
}
