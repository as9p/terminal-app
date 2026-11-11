//! The windowless mode, run as the real binary.
#![cfg(unix)]

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use term_engine::{GridSize, Record, read_recording};

/// Far above what any of these commands needs, so a busy machine does not fail a test.
const LIMIT: Duration = Duration::from_secs(60);

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    took: Duration,
}

/// Kills the binary if the test fails before it exits, so no snowflake or command is left behind.
struct Guard(Child);

impl Drop for Guard {
    fn drop(&mut self) {
        if let Ok(None) = self.0.try_wait() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn snowflake(args: &[&str]) -> Run {
    let mut all = vec!["--headless", "--cols", "80", "--rows", "24"];
    all.extend_from_slice(args);
    run(&all)
}

fn run(args: &[&str]) -> Run {
    let started = Instant::now();
    let child = Command::new(env!("CARGO_BIN_EXE_snowflake"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start snowflake");
    let mut guard = Guard(child);
    let stdout = drain(guard.0.stdout.take().unwrap());
    let stderr = drain(guard.0.stderr.take().unwrap());
    // std has no wait with a timeout, so this polls; the limit is generous.
    let status = loop {
        if let Some(status) = guard.0.try_wait().expect("wait for snowflake") {
            break status;
        }
        assert!(
            started.elapsed() < LIMIT,
            "snowflake {args:?} did not exit in {LIMIT:?}"
        );
        thread::sleep(Duration::from_millis(10));
    };
    let took = started.elapsed();
    Run {
        code: status.code(),
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
        took,
    }
}

fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// A file under the temp folder that is removed when the test ends, pass or fail.
struct TempFile(PathBuf);

impl TempFile {
    fn new(tag: &str) -> TempFile {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("snowflake-test-{}-{tag}-{n}", std::process::id());
        TempFile(std::env::temp_dir().join(name))
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn prints_the_final_screen() {
    let run = snowflake(&["--", "/bin/sh", "-c", r#"printf "hello\r\nworld""#]);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines, ["hello", "world"], "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0));
}

#[test]
fn exit_code_is_the_commands() {
    let run = snowflake(&["--", "/bin/sh", "-c", "exit 7"]);
    assert_eq!(run.code, Some(7), "stderr: {}", run.stderr);
}

#[test]
fn queries_are_answered_through_the_pty() {
    let script = r#"printf "\e[6n"; read -rs -d R reply; print -r -- "got ${reply#*\[}""#;
    let run = snowflake(&["--", "/bin/zsh", "-c", script]);
    assert!(
        run.stdout.contains("got 1;1"),
        "stdout: {:?} stderr: {}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn xtversion_names_the_terminal() {
    let script =
        r#"stty -echo; printf "\e[>0q"; read -rs -t 2 -d '\' reply; print -r -- "got ${(V)reply}""#;
    let run = snowflake(&["--", "/bin/zsh", "-c", script]);
    assert!(
        run.stdout.contains(">|snowflake "),
        "stdout: {:?} stderr: {}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn keys_file_is_typed() {
    let keys = TempFile::new("keys");
    std::fs::write(&keys.0, "300 hi\\r\n600 \\x04\n").unwrap();
    let run = snowflake(&["--keys", keys.path(), "--", "/bin/cat"]);
    let lines: Vec<&str> = run.stdout.lines().collect();
    // A third line is the pty's own echo of the end-of-file key, "^D" on macOS.
    assert_eq!(
        lines.get(..2),
        Some(&["hi", "hi"][..]),
        "stdout: {:?} stderr: {}",
        run.stdout,
        run.stderr
    );
    assert_eq!(run.code, Some(0));
}

#[test]
fn timeout_ends_a_stuck_command() {
    let run = snowflake(&["--timeout", "1", "--", "/bin/cat"]);
    assert_eq!(run.code, Some(124), "stderr: {}", run.stderr);
    assert!(run.took < Duration::from_secs(3), "took {:?}", run.took);
}

#[test]
fn recording_holds_the_output() {
    let file = TempFile::new("rec");
    let run = snowflake(&[
        "--record",
        file.path(),
        "--",
        "/bin/sh",
        "-c",
        "printf hello",
    ]);
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    let records = read_recording(std::fs::File::open(&file.0).unwrap()).unwrap();
    assert_eq!(
        records.first(),
        Some(&Record::Resize {
            ms: 0,
            grid: GridSize { cols: 80, rows: 24 }
        })
    );
    let mut joined = Vec::new();
    for record in &records {
        if let Record::Output { bytes, .. } = record {
            joined.extend_from_slice(bytes);
        }
    }
    let text = String::from_utf8_lossy(&joined);
    assert!(text.contains("hello"), "recorded {text:?}");
}

#[test]
fn environment_names_the_terminal() {
    let run = snowflake(&[
        "--",
        "/bin/sh",
        "-c",
        r#"printf "%s %s" "$TERM" "$TERM_PROGRAM""#,
    ]);
    assert_eq!(
        run.stdout.lines().next(),
        Some("xterm-256color snowflake"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn five_megabyte_write_arrives_whole() {
    let keys = TempFile::new("big");
    let line = format!("{}\\r", "x".repeat(79));
    let mut text = String::from("300 ");
    for _ in 0..65_536 {
        text.push_str(&line);
    }
    text.push_str("\n400 \\x04\n");
    std::fs::write(&keys.0, text).unwrap();
    let run = snowflake(&[
        "--keys",
        keys.path(),
        "--timeout",
        "30",
        "--",
        "/bin/sh",
        "-c",
        "stty -echo; wc -c",
    ]);
    assert!(
        run.stdout.contains("5242880"),
        "stdout: {:?} stderr: {}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn unknown_option_is_a_usage_error() {
    let run = run(&["--bogus"]);
    assert_eq!(run.code, Some(2));
    assert!(
        run.stderr.contains("usage: snowflake"),
        "stderr: {}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "stdout: {}", run.stdout);
}

#[test]
fn output_before_exit_is_not_lost() {
    let script = "i=0; while [ $i -lt 2000 ]; do echo line$i; i=$((i+1)); done";
    let run = snowflake(&["--", "/bin/sh", "-c", script]);
    assert_eq!(
        run.stdout.lines().last(),
        Some("line1999"),
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(0));
}

#[test]
fn bad_record_path_starts_nothing() {
    let marker = TempFile::new("marker");
    let script = format!("touch '{}'", marker.path());
    let run = snowflake(&[
        "--record",
        "/nonexistent-dir/x.rec",
        "--",
        "/bin/sh",
        "-c",
        &script,
    ]);
    assert_eq!(run.code, Some(1), "stderr: {}", run.stderr);
    assert!(!marker.0.exists(), "the command ran");
}
