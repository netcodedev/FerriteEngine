//! The heap as the engine sees it: a global allocator that counts the
//! bytes and blocks alive, around the system's own (`Counting`), and what
//! glibc's malloc holds besides (`malloc_info`).
//!
//! The counters are spread over a few cache lines, picked by the stack the
//! call runs on, so threads allocating at once (chunk builders, physics)
//! do not fight over one line. A block may be freed on another line than
//! it was counted on: only the sum over all lines means anything.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicI64, Ordering::Relaxed},
};

const LINES: usize = 16;

#[repr(align(128))]
struct Line {
    bytes: AtomicI64,
    blocks: AtomicI64,
}

static LINES_: [Line; LINES] = [const { Line { bytes: AtomicI64::new(0), blocks: AtomicI64::new(0) } }; LINES];

/// The line for the calling thread: by where its stack is (threads' stacks
/// lie megabytes apart).
#[inline]
fn line() -> &'static Line {
    let marker = 0u8;
    let at = std::ptr::addr_of!(marker) as usize;
    &LINES_[(at >> 21) % LINES]
}

/// The system allocator, counted.
pub struct Counting;

unsafe impl GlobalAlloc for Counting {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            let l = line();
            l.bytes.fetch_add(layout.size() as i64, Relaxed);
            l.blocks.fetch_add(1, Relaxed);
        }
        p
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            let l = line();
            l.bytes.fetch_add(layout.size() as i64, Relaxed);
            l.blocks.fetch_add(1, Relaxed);
        }
        p
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        let l = line();
        l.bytes.fetch_sub(layout.size() as i64, Relaxed);
        l.blocks.fetch_sub(1, Relaxed);
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            line().bytes.fetch_add(new_size as i64 - layout.size() as i64, Relaxed);
        }
        p
    }
}

/// Bytes and blocks alive on the heap now (Rust's allocations; C libraries
/// that call malloc themselves, such as the graphics driver, not counted).
pub fn live() -> (u64, u64) {
    let (mut bytes, mut blocks) = (0i64, 0i64);
    for l in &LINES_ {
        bytes += l.bytes.load(Relaxed);
        blocks += l.blocks.load(Relaxed);
    }
    (bytes.max(0) as u64, blocks.max(0) as u64)
}

/// What glibc's malloc holds, over all its arenas: (in use, free but kept,
/// in blocks of their own from mmap). Everything malloc'd counts here, the
/// driver's and C libraries' too. `None` elsewhere than glibc.
pub fn malloc_info() -> Option<MallocInfo> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        #[repr(C)]
        struct Mallinfo2 {
            arena: usize,
            ordblks: usize,
            smblks: usize,
            hblks: usize,
            hblkhd: usize,
            usmblks: usize,
            fsmblks: usize,
            uordblks: usize,
            fordblks: usize,
            keepcost: usize,
        }
        extern "C" {
            fn mallinfo2() -> Mallinfo2;
        }
        // Walks every arena under its lock: fine a few times a minute.
        let m = unsafe { mallinfo2() };
        Some(MallocInfo {
            in_use: (m.uordblks + m.hblkhd) as u64,
            free_kept: m.fordblks as u64,
            mapped: m.hblkhd as u64,
            mapped_blocks: m.hblks as u64,
        })
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        None
    }
}

/// Blocks from this size up come from their own mmap and go back to the
/// system when freed (`tune_malloc`).
pub const MMAP_THRESHOLD: i32 = 256 << 10;

/// Set glibc's malloc up for the engine: big blocks from their own mmap.
///
/// By default glibc raises that threshold up to 32 MB as big blocks are
/// freed, so chunk meshes, collider and scatter lists (hundreds of KB to
/// MB) end up in the arenas of the threads that made them, and stay there
/// freed: three minutes of flying over the demo grew the process by
/// 1.67 GB of memory nothing used any more (docs/memory-map.md). With a
/// fixed threshold it grew by 0.1 GB (the world's caches filling), and
/// started 1 GB smaller. A periodic `malloc_trim` did nearly as well but
/// held the arenas' locks for up to 42 ms. Nothing elsewhere than glibc.
pub fn tune_malloc() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        const M_MMAP_THRESHOLD: i32 = -3;
        extern "C" {
            fn mallopt(param: i32, value: i32) -> i32;
        }
        if unsafe { mallopt(M_MMAP_THRESHOLD, MMAP_THRESHOLD) } != 1 {
            log::warn!("malloc: could not set the mmap threshold");
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MallocInfo {
    /// Handed out and not freed (arenas and own blocks).
    pub in_use: u64,
    /// Freed but kept in the arenas: in the process's memory, not in use.
    pub free_kept: u64,
    /// In blocks of their own (big allocations, from mmap).
    pub mapped: u64,
    pub mapped_blocks: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_what_lives() {
        // Other test threads allocate at the same time: allow for them.
        let (before, _) = live();
        let block = vec![0u8; 64 << 20];
        let (during, _) = live();
        drop(block);
        let (after, _) = live();
        assert!(during >= before + (60 << 20), "{before} -> {during}");
        assert!(after + (60 << 20) <= during, "{during} -> {after}");
    }
}
