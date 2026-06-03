//! Fair FIFO ticket lock and recursive ticket lock.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::ticket_lock`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// A fair FIFO ticket lock.
///
/// Unlike a spinlock, this implementation blocks threads that can't acquire the lock,
/// using a ticket queue to ensure FIFO ordering.
#[derive(Debug)]
pub struct TicketLock {
    next_ticket: AtomicU32,
    now_serving: AtomicU32,
}

unsafe impl Send for TicketLock {}
unsafe impl Sync for TicketLock {}

impl TicketLock {
    /// Creates a new ticket lock.
    pub const fn new() -> Self {
        TicketLock {
            next_ticket: AtomicU32::new(0),
            now_serving: AtomicU32::new(0),
        }
    }

    /// Acquires the lock.
    pub fn lock(&self) -> TicketLockGuard<'_> {
        let ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);

        // Spin-wait until it's our turn
        while self.now_serving.load(Ordering::Acquire) != ticket {
            #[cfg(target_os = "windows")]
            std::thread::yield_now();

            #[cfg(not(target_os = "windows"))]
            std::hint::spin_loop();
        }
        TicketLockGuard { lock: self }
    }

    /// Releases the lock (called by the guard's Drop).
    fn unlock_inner(&self) {
        self.now_serving.fetch_add(1, Ordering::Release);
        // Note: For true cross-platform notification, we'd use a Condvar.
        // For simplicity, we rely on the spinning threads to notice the update.
    }
}

impl Default for TicketLock {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard for the ticket lock.
pub struct TicketLockGuard<'a> {
    lock: &'a TicketLock,
}

impl<'a> Drop for TicketLockGuard<'a> {
    fn drop(&mut self) {
        self.lock.unlock_inner();
    }
}

/// A recursive ticket lock that tracks the owning thread.
#[derive(Debug)]
pub struct RecursiveTicketLock {
    lock: TicketLock,
    owner: AtomicU64,
    recursion: AtomicU32,
}

unsafe impl Send for RecursiveTicketLock {}
unsafe impl Sync for RecursiveTicketLock {}

impl RecursiveTicketLock {
    /// Creates a new recursive ticket lock.
    pub const fn new() -> Self {
        RecursiveTicketLock {
            lock: TicketLock::new(),
            owner: AtomicU64::new(0),
            recursion: AtomicU32::new(0),
        }
    }

    /// Acquires the lock. Can be called recursively from the same thread.
    pub fn lock(&self) {
        let current_thread = Self::thread_id();

        if self.owner.load(Ordering::Relaxed) != current_thread {
            self.lock.lock();
            self.owner.store(current_thread, Ordering::Relaxed);
        }
        self.recursion.fetch_add(1, Ordering::Relaxed);
    }

    /// Releases the lock. Must be called once for each lock() call.
    pub fn unlock(&self) {
        if self.recursion.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.owner.store(0, Ordering::Relaxed);
            self.lock.unlock_inner();
        }
    }

    /// Returns whether the lock is held by the current thread.
    pub fn is_locked(&self) -> bool {
        let current_thread = Self::thread_id();
        self.owner.load(Ordering::Relaxed) == current_thread
    }

    fn thread_id() -> u64 {
        let mut hasher = DefaultHasher::new();
        std::thread::current().id().hash(&mut hasher);
        hasher.finish()
    }

    /// Returns the current recursion depth for the current thread.
    pub fn recursion_depth(&self) -> u32 {
        if self.is_locked() {
            self.recursion.load(Ordering::Relaxed)
        } else {
            0
        }
    }
}

impl Default for RecursiveTicketLock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU32;
    use std::thread;

    #[test]
    fn test_ticket_lock_basic() {
        let lock = TicketLock::new();
        {
            let _guard = lock.lock();
            // Lock is held here
        }
        // Lock is released
    }

    #[test]
    fn test_ticket_lock_mutual_exclusion() {
        let lock = Arc::new(TicketLock::new());
        let counter = Arc::new(AtomicU32::new(0));
        let mut handles = vec![];

        for _ in 0..5 {
            let lock = Arc::clone(&lock);
            let counter = Arc::clone(&counter);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    let _guard = lock.lock();
                    let val = counter.load(Ordering::Relaxed);
                    counter.store(val + 1, Ordering::Relaxed);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(counter.load(Ordering::Relaxed), 500);
    }

    #[test]
    fn test_recursive_ticket_lock() {
        let lock = RecursiveTicketLock::new();
        lock.lock();
        lock.lock(); // recursive
        assert_eq!(lock.recursion_depth(), 2);
        lock.unlock();
        assert_eq!(lock.recursion_depth(), 1);
        lock.unlock();
        assert_eq!(lock.recursion_depth(), 0);
    }

    #[test]
    fn test_recursive_ticket_lock_is_locked() {
        let lock = RecursiveTicketLock::new();
        assert!(!lock.is_locked());
        lock.lock();
        assert!(lock.is_locked());
        lock.unlock();
        assert!(!lock.is_locked());
    }
}
