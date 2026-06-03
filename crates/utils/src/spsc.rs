//! Single-producer single-consumer queue.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::spsc_queue`.

use std::sync::Mutex;

/// A simple SPSC queue for transferring data between threads.
///
/// This is a bounded queue that blocks when full or empty.
pub struct SpscQueue<T> {
    buffer: Mutex<Vec<T>>,
    capacity: usize,
    full: std::sync::Condvar,
    empty: std::sync::Condvar,
}

unsafe impl<T: Send> Send for SpscQueue<T> {}
unsafe impl<T: Send> Sync for SpscQueue<T> {}

impl<T> SpscQueue<T> {
    /// Creates a new SPSC queue with the given capacity.
    pub fn new(capacity: usize) -> Self {
        SpscQueue {
            buffer: Mutex::new(Vec::with_capacity(capacity)),
            capacity,
            full: std::sync::Condvar::new(),
            empty: std::sync::Condvar::new(),
        }
    }

    /// Pushes a value into the queue, blocking if full.
    pub fn push(&self, value: T) {
        let mut buffer = self.buffer.lock().unwrap();
        while buffer.len() >= self.capacity {
            buffer = self.full.wait(buffer).unwrap();
        }
        buffer.push(value);
        self.empty.notify_one();
    }

    /// Pops a value from the queue, blocking if empty.
    pub fn pop(&self) -> T {
        let mut buffer = self.buffer.lock().unwrap();
        while buffer.is_empty() {
            buffer = self.empty.wait(buffer).unwrap();
        }
        let value = buffer.remove(0);
        self.full.notify_one();
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_push_pop() {
        let queue = SpscQueue::new(10);
        queue.push(42);
        assert_eq!(queue.pop(), 42);
    }

    #[test]
    fn test_multiple_push_pop() {
        let queue = Arc::new(SpscQueue::new(5));

        let q = Arc::clone(&queue);
        let handle = thread::spawn(move || {
            for i in 0..10 {
                q.push(i);
            }
        });

        // Start consuming in a separate thread to avoid deadlock
        let receiver = Arc::clone(&queue);
        let results = Arc::new(std::sync::Mutex::new(Vec::new()));
        let r = Arc::clone(&results);
        let consumer = thread::spawn(move || {
            for _ in 0..10 {
                r.lock().unwrap().push(receiver.pop());
            }
        });

        handle.join().unwrap();
        consumer.join().unwrap();

        let results = results.lock().unwrap();
        for (i, &val) in results.iter().enumerate() {
            assert_eq!(val, i);
        }
    }
}
