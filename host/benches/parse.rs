use std::hint::black_box;
use std::time::Instant;

use term_host::{Screen, Size};

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

fn main() {
    measure_memory_and_snapshot();
    measure_speed();
}
