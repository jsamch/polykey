//! The global allocator of the binary: the system allocator, but every block is overwritten
//! with zeros before it is freed.
//!
//! `Zeroizing` wipes the buffers this program owns. It cannot reach the copies that libraries
//! make and free on their own: the 128 MiB scrypt work area and its first block, which is a
//! single PBKDF2-SHA256 of the passcode and so a cheap oracle for guessing it; egui's per-frame
//! copy of every text field (`prev_text`), its undo history, the galleys of laid out text and
//! the `String` of every typed character; `rpassword`'s line buffer while it grows; the decoded
//! QR payloads inside `rxing`; and the old buffer whenever any `String` or `Vec` grows. With
//! this allocator none of those outlives its `free`. A copy still alive (on screen, in a cache
//! of the current frame) is not covered; nor is the stack (see `polykey_core::wipe`).
//!
//! `realloc` always moves the block, so the old block goes through `dealloc` and is wiped; the
//! system's own `realloc` could move or shrink it without wiping.

#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::ptr;

/// Wraps an allocator so that every block is zeroed before it is freed.
pub struct WipeOnFree<A = System>(pub A);

/// Zeroes `len` bytes at `ptr`. The pointer then goes through `black_box`, which the compiler
/// must treat as possibly reading the block, so the writes cannot be removed as dead stores
/// even though the block is freed right after.
///
/// # Safety
/// `ptr` must be valid for writes of `len` bytes.
unsafe fn wipe(ptr: *mut u8, len: usize) {
    // SAFETY: the caller guarantees that `ptr` is valid for `len` bytes.
    unsafe { ptr::write_bytes(ptr, 0, len) };
    std::hint::black_box(ptr);
}

// SAFETY: every method forwards to the inner allocator with the same layouts it was given;
// `realloc` is built from `alloc`, a copy of the smaller size and `dealloc`, which is the
// documented default behaviour of `GlobalAlloc::realloc`.
unsafe impl<A: GlobalAlloc> GlobalAlloc for WipeOnFree<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: same contract as the caller's.
        unsafe { self.0.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: same contract as the caller's.
        unsafe { self.0.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` is a live block of `layout.size()` bytes from this allocator.
        unsafe {
            wipe(ptr, layout.size());
            self.0.dealloc(ptr, layout);
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller guarantees `new_size`, rounded up to `layout.align()`, does not
        // overflow `isize`, so the layout is valid.
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        // SAFETY: `new_size` is non-zero by the caller's contract.
        let new = unsafe { self.0.alloc(new_layout) };
        if !new.is_null() {
            // SAFETY: both blocks are live, do not overlap and hold at least the copied size.
            unsafe {
                ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                self.dealloc(ptr, layout);
            }
        }
        new
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The system allocator, counting the blocks that reach `dealloc` with a nonzero byte.
    #[derive(Default)]
    struct Checking {
        dirty: AtomicUsize,
        freed: AtomicUsize,
    }

    unsafe impl GlobalAlloc for Checking {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            let bytes = unsafe { std::slice::from_raw_parts(ptr, layout.size()) };
            if bytes.iter().any(|&b| b != 0) {
                self.dirty.fetch_add(1, Ordering::Relaxed);
            }
            self.freed.fetch_add(1, Ordering::Relaxed);
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    fn fill(ptr: *mut u8, len: usize, byte: u8) {
        unsafe { ptr::write_bytes(ptr, byte, len) };
    }

    #[test]
    fn the_checker_sees_a_block_freed_with_its_contents() {
        let a = Checking::default();
        let layout = Layout::from_size_align(64, 8).unwrap();
        unsafe {
            let p = a.alloc(layout);
            fill(p, 64, 0xA5);
            a.dealloc(p, layout);
        }
        assert_eq!(a.dirty.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn freed_blocks_are_zero() {
        let a = WipeOnFree(Checking::default());
        for size in [1, 31, 64, 4096, 1 << 20] {
            let layout = Layout::from_size_align(size, 16).unwrap();
            unsafe {
                let p = a.alloc(layout);
                fill(p, size, 0x5A);
                a.dealloc(p, layout);
            }
        }
        assert_eq!(a.0.freed.load(Ordering::Relaxed), 5);
        assert_eq!(a.0.dirty.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn growing_and_shrinking_keep_the_data_and_wipe_the_old_block() {
        let a = WipeOnFree(Checking::default());
        let small = Layout::from_size_align(16, 8).unwrap();
        unsafe {
            let p = a.alloc(small);
            fill(p, 16, 0x11);
            let grown = a.realloc(p, small, 4096);
            assert!(std::slice::from_raw_parts(grown, 16)
                .iter()
                .all(|&b| b == 0x11));
            fill(grown, 4096, 0x22);
            let big = Layout::from_size_align(4096, 8).unwrap();
            let shrunk = a.realloc(grown, big, 8);
            assert!(std::slice::from_raw_parts(shrunk, 8)
                .iter()
                .all(|&b| b == 0x22));
            a.dealloc(shrunk, Layout::from_size_align(8, 8).unwrap());
        }
        assert_eq!(a.0.freed.load(Ordering::Relaxed), 3);
        assert_eq!(a.0.dirty.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_zeroed_allocation_is_zero() {
        let a = WipeOnFree(System);
        let layout = Layout::from_size_align(256, 8).unwrap();
        unsafe {
            let p = a.alloc_zeroed(layout);
            assert!(std::slice::from_raw_parts(p, 256).iter().all(|&b| b == 0));
            a.dealloc(p, layout);
        }
    }
}
