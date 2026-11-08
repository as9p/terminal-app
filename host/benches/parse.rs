use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use term_host::{Screen, Size};

/// Counts live bytes, because resident size cannot tell memory the screen still holds from
/// memory it freed: the system allocator keeps freed pages around for reuse.
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

fn live_mb() -> f64 {
    LIVE.load(Ordering::Relaxed) as f64 / MB
}

const SIZE: Size = Size {
    cols: 200,
    rows: 50,
};
const SCROLLBACK: usize = 20_000;
const TOTAL: usize = 30 * 1024 * 1024;
const MB: f64 = 1024.0 * 1024.0;
/// The size a pty read tends to hand over at once.
const CHUNK: usize = 64 * 1024;
const PASS_MB_PER_S: f64 = 80.0;

/// A fixed seed makes every run parse the same bytes, so runs can be compared.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, limit: u64) -> u64 {
        self.next() % limit
    }
}

/// Appends colored words until the line holds `width` printed characters.
fn push_line(rng: &mut Rng, width: usize, out: &mut Vec<u8>) {
    let mut printed = 0;
    while printed < width {
        match rng.below(4) {
            0 => {
                let color = format!("\x1b[38;5;{}m", rng.below(256));
                out.extend_from_slice(color.as_bytes());
            }
            1 => {
                let color = format!(
                    "\x1b[38;2;{};{};{}m",
                    rng.below(256),
                    rng.below(256),
                    rng.below(256)
                );
                out.extend_from_slice(color.as_bytes());
            }
            2 => out.extend_from_slice(b"\x1b[1m"),
            _ => out.extend_from_slice(b"\x1b[m"),
        }
        let word = (rng.below(12) as usize + 1).min(width - printed);
        for _ in 0..word {
            out.push(b'a' + rng.below(26) as u8);
        }
        printed += word;
        if printed < width {
            out.push(b' ');
            printed += 1;
        }
    }
    out.extend_from_slice(b"\x1b[m\r\n");
}

fn generate(total: usize) -> Vec<u8> {
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    let mut out = Vec::with_capacity(total + 4096);
    while out.len() < total {
        // Some lines are longer than the window, so wrapping gets parsed too.
        let width = 40 + rng.below(240) as usize;
        push_line(&mut rng, width, &mut out);
    }
    out.truncate(total);
    out
}

/// Peak resident size in MiB. The kernel reports bytes on macOS and KiB on Linux.
#[cfg(unix)]
fn peak_rss_mb() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage fills in the struct it is pointed at, and zeroed is a valid rusage.
    let usage = unsafe {
        libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr());
        usage.assume_init()
    };
    let unit = if cfg!(target_os = "macos") {
        1.0
    } else {
        1024.0
    };
    usage.ru_maxrss as f64 * unit / MB
}

#[cfg(not(unix))]
fn peak_rss_mb() -> f64 {
    0.0
}

/// Runs first because peak resident size only ever rises, and the speed run's 30 MB
/// buffer would hide the screen's own share.
fn measure_memory_and_snapshot() {
    let before = peak_rss_mb();
    let mut screen = Screen::new(SIZE, SCROLLBACK);
    let mut rng = Rng(0x0dd_ba11_c0ff_ee42);
    let mut line = Vec::new();
    for _ in 0..SCROLLBACK + usize::from(SIZE.rows) {
        line.clear();
        push_line(&mut rng, usize::from(SIZE.cols), &mut line);
        screen.feed(&line);
    }
    let filled = peak_rss_mb();
    assert_eq!(screen.vt().scrollback_len(), SCROLLBACK);

    let start = Instant::now();
    let snapshot = screen.snapshot();
    let elapsed = start.elapsed();
    let after = peak_rss_mb();
    println!(
        "memory: peak {filled:.1} MiB with {SCROLLBACK} lines at {} columns ({:.1} MiB above start at {before:.1}), peak {after:.1} MiB after one snapshot",
        SIZE.cols,
        filled - before,
    );
    println!(
        "snapshot: {:.1} ms, {:.1} MiB",
        elapsed.as_secs_f64() * 1000.0,
        snapshot.len() as f64 / MB,
    );
    black_box(snapshot);
}

fn measure_speed() {
    let data = generate(TOTAL);
    let mut screen = Screen::new(SIZE, SCROLLBACK);
    let start = Instant::now();
    for chunk in data.chunks(CHUNK) {
        screen.feed(black_box(chunk));
    }
    let elapsed = start.elapsed().as_secs_f64();
    let speed = data.len() as f64 / MB / elapsed;
    let verdict = if speed >= PASS_MB_PER_S {
        "pass"
    } else {
        "under"
    };
    println!(
        "speed: {speed:.1} MB/s, {:.0} MiB in {elapsed:.3} s, {verdict} against {PASS_MB_PER_S} MB/s",
        data.len() as f64 / MB,
    );
    black_box(screen.vt().scrollback_len());
}

/// Resident size right now, where `peak_rss_mb` can never show memory being given back.
fn current_rss_mb() -> f64 {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    let kib: f64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("ps prints a number");
    kib * 1024.0 / MB
}

/// A shell's output is mostly short lines, which vt100 stores trimmed. A snapshot must not
/// leave them padded, and the memory it uses for the moment should be reused by the next
/// one, so several sessions are attached in turn. Run alone with the `short` argument, to
/// keep the other measurements out of the resident size.
fn measure_short_lines() {
    const SESSIONS: usize = 4;
    let before = current_rss_mb();
    let mut rng = Rng(0x0dd_ba11_c0ff_ee42);
    let mut line = Vec::new();
    let mut screens = Vec::new();
    for _ in 0..SESSIONS {
        let mut screen = Screen::new(SIZE, SCROLLBACK);
        for _ in 0..SCROLLBACK + usize::from(SIZE.rows) {
            line.clear();
            push_line(&mut rng, 40, &mut line);
            screen.feed(&line);
        }
        screens.push(screen);
    }
    let live_filled = live_mb();
    let rss_filled = current_rss_mb() - before;

    let start = Instant::now();
    let mut size = 0;
    for screen in &mut screens {
        let snapshot = screen.snapshot();
        size = snapshot.len();
        black_box(snapshot);
    }
    let elapsed = start.elapsed().as_secs_f64() * 1000.0 / SESSIONS as f64;
    println!(
        "short lines, {SESSIONS} sessions of {SCROLLBACK} lines at 40 characters: live {live_filled:.1} MiB before snapshots, {:.1} MiB after; resident {rss_filled:.1} MiB before, {:.1} MiB after; each snapshot {elapsed:.1} ms and {:.1} MiB",
        live_mb(),
        current_rss_mb() - before,
        size as f64 / MB,
    );
    black_box(screens);
}

fn main() {
    if std::env::args().any(|arg| arg == "short") {
        measure_short_lines();
        return;
    }
    measure_memory_and_snapshot();
    measure_speed();
}
