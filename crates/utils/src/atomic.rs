//! Atomic wait/notify primitives.

use std::sync::{Condvar, Mutex as StdMutex};
use std::time::Duration;

/// A notification gate for thread synchronization.
#[derive(Debug)]
pub struct NotificationGate {
    mutex: StdMutex<bool>,
    condvar: Condvar,
}

impl NotificationGate {
    /// Creates a new gate in the "unsignaled" state.
    pub const fn new() -> Self {
        NotificationGate {
            mutex: StdMutex::new(false),
            condvar: Condvar::new(),
        }
    }

    /// Signals the gate, waking any waiters.
    pub fn notify(&self) {
        let mut signaled = self.mutex.lock().unwrap();
        *signaled = true;
        self.condvar.notify_all();
    }

    /// Waits until the gate is signaled.
    pub fn wait(&self) {
        let mut signaled = self.mutex.lock().unwrap();
        while !*signaled {
            signaled = self.condvar.wait(signaled).unwrap();
        }
    }

    /// Waits with a timeout. Returns true if signaled, false if timed out.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let mut signaled = self.mutex.lock().unwrap();
        while !*signaled {
            let (guard, timeout_result) = self.condvar.wait_timeout(signaled, timeout).unwrap();
            if !timeout_result.timed_out() {
                if *guard {
                    return true;
                }
                signaled = guard;
            } else {
                return false;
            }
        }
        true
    }

    /// Returns whether the gate has been signaled.
    pub fn is_signaled(&self) -> bool {
        *self.mutex.lock().unwrap()
    }
}

impl Default for NotificationGate {
    fn default() -> Self {
        Self::new()
    }
}

/// Wait/notify for atomic values using a condvar.
#[derive(Debug)]
pub struct AtomicWaitNotify<T> {
    value: std::sync::Mutex<T>,
    condvar: Condvar,
}

unsafe impl<T: Send> Send for AtomicWaitNotify<T> {}
unsafe impl<T: Send> Sync for AtomicWaitNotify<T> {}

impl<T: PartialEq + Send> AtomicWaitNotify<T> {
    /// Creates a new wait/notify for the given initial value.
    pub const fn new(value: T) -> Self {
        AtomicWaitNotify {
            value: std::sync::Mutex::new(value),
            condvar: Condvar::new(),
        }
    }

    /// Sets the value and notifies all waiters.
    pub fn store(&self, value: T) {
        let mut v = self.value.lock().unwrap();
        *v = value;
        self.condvar.notify_all();
    }

    /// Loads the current value.
    pub fn load(&self) -> T
    where
        T: Copy,
    {
        *self.value.lock().unwrap()
    }

    /// Waits until the value equals the expected value.
    pub fn wait_for(&self, expected: T)
    where
        T: Copy,
    {
        let mut v = self.value.lock().unwrap();
        while *v != expected {
            v = self.condvar.wait(v).unwrap();
        }
    }

    /// Waits with a timeout until the value equals the expected value.
    /// Returns true if the value matched, false if timed out.
    pub fn wait_for_timeout(&self, expected: T, timeout: Duration) -> bool
    where
        T: Copy,
    {
        let mut v = self.value.lock().unwrap();
        while *v != expected {
            let (guard, result) = self.condvar.wait_timeout(v, timeout).unwrap();
            if result.timed_out() {
                return false;
            }
            v = guard;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_notification_gate_basic() {
        let gate = NotificationGate::new();
        assert!(!gate.is_signaled());

        gate.notify();
        assert!(gate.is_signaled());
    }

    #[test]
    fn test_notification_gate_wait() {
        let gate = Arc::new(NotificationGate::new());
        let gate_clone = Arc::clone(&gate);

        let handle = thread::spawn(move || {
            gate_clone.wait();
        });

        thread::sleep(Duration::from_millis(10));
        gate.notify();
        handle.join().unwrap();
    }

    #[test]
    fn test_notification_gate_timeout() {
        let gate = NotificationGate::new();
        let result = gate.wait_timeout(Duration::from_millis(10));
        assert!(!result);
    }

    #[test]
    fn test_notification_gate_timeout_succeeds() {
        let gate = Arc::new(NotificationGate::new());
        let gate_clone = Arc::clone(&gate);

        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            gate_clone.notify();
        });

        let result = gate.wait_timeout(Duration::from_secs(1));
        assert!(result);
        handle.join().unwrap();
    }

    #[test]
    fn test_atomic_wait_notify() {
        let awn = Arc::new(AtomicWaitNotify::new(0i32));
        let awn_clone = Arc::clone(&awn);

        let handle = thread::spawn(move || {
            awn_clone.wait_for(42);
        });

        thread::sleep(Duration::from_millis(10));
        awn.store(42);
        handle.join().unwrap();
    }
}
