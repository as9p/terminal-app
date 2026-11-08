//! Alone in its own test binary because it counts every allocation in the process, and tests
//! running beside it would disturb the count.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use term_host::{Screen, Size};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call is passed straight to the system allocator, only counted on the way.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(new_size, Ordering::Relaxed);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn snapshot_leaves_short_lines_trimmed() {
    const LINES: usize = 2000;
    let mut screen = Screen::new(
        Size {
            cols: 200,
            rows: 50,
        },
        LINES,
    );
    for i in 0..LINES + 50 {
        screen.feed(format!("line {i}\r\n").as_bytes());
    }

    let before = LIVE.load(Ordering::Relaxed);
    let snapshot = screen.snapshot();
    assert!(!snapshot.is_empty());
    drop(snapshot);
    let grown = LIVE.load(Ordering::Relaxed).saturating_sub(before);

    // Padding these rows to 200 columns would add about 14 MiB.
    assert!(grown < 1024 * 1024, "the snapshot kept {grown} bytes");
}
