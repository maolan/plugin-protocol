use std::sync::atomic::{AtomicU32, Ordering};

/// Lock-free single-producer / single-consumer ring buffer backed by a fixed
/// array in shared memory.
///
/// `T` must be `Copy` so that reads/writes are safe without destructors.
/// The capacity must be a power of two so that modulo can be done with a mask.
pub struct RingBuffer<T> {
    buffer: *mut T,
    write_idx: *mut AtomicU32,
    read_idx: *mut AtomicU32,
    mask: u32,
}

unsafe impl<T: Send> Send for RingBuffer<T> {}
unsafe impl<T: Sync> Sync for RingBuffer<T> {}

impl<T: Copy> RingBuffer<T> {
    /// # Safety
    ///
    /// `buffer` must point to `capacity` valid, initialized slots.
    /// `write_idx` and `read_idx` must point to distinct `AtomicU32`s.
    /// `capacity` must be a power of two.
    pub unsafe fn new(
        buffer: *mut T,
        write_idx: *mut AtomicU32,
        read_idx: *mut AtomicU32,
        capacity: usize,
    ) -> Self {
        assert!(
            capacity.is_power_of_two(),
            "RingBuffer capacity must be a power of two"
        );
        Self {
            buffer,
            write_idx,
            read_idx,
            mask: (capacity - 1) as u32,
        }
    }

    /// Attempt to push a single event. Returns `true` on success, `false` if
    /// the buffer is full.
    pub fn push(&self, event: T) -> bool {
        let write = unsafe { (*self.write_idx).load(Ordering::Relaxed) };
        let read = unsafe { (*self.read_idx).load(Ordering::Acquire) };
        let count = write.wrapping_sub(read);
        if count as usize >= (self.mask as usize) {
            return false;
        }
        let idx = (write & self.mask) as usize;
        unsafe {
            self.buffer.add(idx).write(event);
        }
        unsafe {
            (*self.write_idx).store(write.wrapping_add(1), Ordering::Release);
        }
        true
    }

    /// Pop a single event. Returns `None` if the buffer is empty.
    pub fn pop(&self) -> Option<T> {
        let read = unsafe { (*self.read_idx).load(Ordering::Relaxed) };
        let write = unsafe { (*self.write_idx).load(Ordering::Acquire) };
        if read == write {
            return None;
        }
        let idx = (read & self.mask) as usize;
        let event = unsafe { self.buffer.add(idx).read() };
        unsafe {
            (*self.read_idx).store(read.wrapping_add(1), Ordering::Release);
        }
        Some(event)
    }

    /// Returns the number of readable slots.
    pub fn len(&self) -> usize {
        let write = unsafe { (*self.write_idx).load(Ordering::Acquire) };
        let read = unsafe { (*self.read_idx).load(Ordering::Acquire) };
        write.wrapping_sub(read) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn overflow_drops_excess() {
        let mut buf = vec![0u32; 8];
        let write = AtomicU32::new(0);
        let read = AtomicU32::new(0);
        let rb = unsafe {
            RingBuffer::new(
                buf.as_mut_ptr(),
                &write as *const AtomicU32 as *mut AtomicU32,
                &read as *const AtomicU32 as *mut AtomicU32,
                8,
            )
        };

        // Fill the buffer to capacity (7 usable slots in an 8-slot ring).
        for i in 0..7 {
            assert!(rb.push(i), "push {i} should succeed");
        }

        // Next push must fail (full).
        assert!(!rb.push(99), "overflow push should be rejected");

        // Read back the 7 items in order.
        for i in 0..7 {
            assert_eq!(rb.pop(), Some(i), "should read item {i}");
        }

        assert!(rb.is_empty());
    }
}
