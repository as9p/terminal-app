//! What a full scrollback costs. Stage B plans forty terminals around this figure, so it is
//! measured with a counting allocator rather than worked out from the library's cell size.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use term_engine::{GridSize, SCROLLBACK_LINES, Screen, Theme};

/// Counts the bytes that are allocated and not yet freed.
struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            LIVE.fetch_add(new_size as isize - layout.size() as isize, Ordering::Relaxed);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn live() -> isize {
    LIVE.load(Ordering::Relaxed)
}

const COLS: u16 = 120;
const ROWS: u16 = 40;
const LINES_FED: usize = 20_100;

/// A screen at 120 by 40 with its scrollback full: every line 120 visible characters, each in a
/// different colour from the line before.
fn full_screen() -> Screen {
    let mut screen = Screen::new(GridSize { cols: COLS, rows: ROWS }, &Theme::builtin().palette);
    let text: String = (0..usize::from(COLS)).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    for n in 0..LINES_FED {
        screen.feed(format!("\x1b[{}m{text}\r\n", 31 + n % 7).as_bytes());
    }
    screen
}

#[test]
fn full_scrollback_memory_is_bounded() {
    let before = live();
    let first = full_screen();
    let one_terminal_bytes = live() - before;
    assert_eq!(first.history_len(), SCROLLBACK_LINES);

    let cells = f64::from(COLS) * (SCROLLBACK_LINES + usize::from(ROWS)) as f64;
    let bytes_per_cell = one_terminal_bytes as f64 / cells;
    println!("MEMORY one_terminal_bytes={one_terminal_bytes} bytes_per_cell={bytes_per_cell:.2}");
    assert!(bytes_per_cell <= 32.0, "{bytes_per_cell:.2} bytes per cell");
    drop(first);

    let before = live();
    let four: Vec<Screen> = (0..4).map(|_| full_screen()).collect();
    let four_terminals_bytes = live() - before;
    println!("MEMORY four_terminals_bytes={four_terminals_bytes}");
    let expected = 4.0 * one_terminal_bytes as f64;
    assert!(
        (four_terminals_bytes as f64 - expected).abs() <= expected * 0.1,
        "four terminals took {four_terminals_bytes} bytes, four times one is {expected}"
    );
    drop(four);
}
