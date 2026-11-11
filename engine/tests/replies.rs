//! What the view answers when a program asks the terminal about itself.

use alacritty_terminal::vte::{Params, Parser, Perform};
use term_engine::{GridSize, Platform, TerminalView, Theme};

const QUERY: &[u8] = b"\x1b[>0q";

/// Counts the XTVERSION queries the parser itself reads, `CSI > q` or `CSI > 0 q`, so the scan
/// is checked against the parser rather than against a reading of it.
struct Queries(usize);

impl Perform for Queries {
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        let zero = params.len() <= 1 && params.iter().all(|param| param == [0]);
        if action == 'q' && intermediates == b">" && !ignore && zero {
            self.0 += 1;
        }
    }
}

fn view() -> TerminalView {
    TerminalView::new(
        GridSize { cols: 80, rows: 24 },
        &Theme::builtin(),
        Platform::Mac,
    )
}

fn reply(identity: &str) -> Vec<u8> {
    format!("\x1bP>|{identity}\x1b\\").into_bytes()
}

fn default_reply() -> Vec<u8> {
    reply(&format!("term-engine {}", env!("CARGO_PKG_VERSION")))
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// `stream` fed whole, cut once at every byte, and one byte at a time.
fn ways_to_feed(stream: &[u8]) -> Vec<Vec<&[u8]>> {
    let mut ways = vec![vec![stream]];
    ways.extend((1..stream.len()).map(|cut| vec![&stream[..cut], &stream[cut..]]));
    ways.push(stream.chunks(1).collect());
    ways
}

/// Feeds `pieces` to a view and to a bare parser: what the view sent back, and how many
/// XTVERSION queries the parser read.
fn replies_and_queries(pieces: &[&[u8]]) -> (Vec<u8>, usize) {
    let mut view = view();
    let mut parser: Parser = Parser::new();
    let mut queries = Queries(0);
    for piece in pieces {
        view.feed(piece);
        parser.advance(&mut queries, piece);
    }
    (view.take_output(), queries.0)
}

/// Every way of feeding `stream` gets `expected` replies, as many as the parser reads queries.
fn assert_replies(stream: &[u8], expected: usize) {
    for pieces in ways_to_feed(stream) {
        let (output, queries) = replies_and_queries(&pieces);
        assert_eq!(queries, expected, "the parser reads {pieces:?} otherwise");
        assert_eq!(count(&output, b"\x1bP>|"), expected, "{pieces:?}");
        assert_eq!(count(&output, &default_reply()), expected, "{pieces:?}");
    }
}

#[test]
fn xtversion_is_answered() {
    let mut view = view();
    view.feed(QUERY);
    assert_eq!(view.take_output(), default_reply());
}

#[test]
fn xtversion_without_a_parameter_is_answered() {
    let mut view = view();
    view.feed(b"\x1b[>q");
    assert_eq!(view.take_output(), default_reply());
}

#[test]
fn other_parameters_get_no_reply() {
    let mut view = view();
    view.feed(b"\x1b[>1q");
    assert_eq!(view.take_output(), b"");
    view.feed(b"\x1b[>0;1q");
    assert_eq!(view.take_output(), b"");
    view.feed(b"\x1b[>00q");
    assert_eq!(view.take_output(), b"");
    // Secondary device attributes may get the engine's own answer, but never this one.
    view.feed(b"\x1b[>0c");
    assert_eq!(count(&view.take_output(), b"\x1bP>|"), 0);
}

#[test]
fn xtversion_split_at_every_byte_is_answered_once() {
    for split in 1..QUERY.len() {
        let mut view = view();
        view.feed(&QUERY[..split]);
        view.feed(&QUERY[split..]);
        assert_eq!(view.take_output(), default_reply(), "split at {split}");
    }
    for query in [QUERY, b"\x1b[>q"] {
        let mut view = view();
        for byte in query {
            view.feed(&[*byte]);
        }
        assert_eq!(
            view.take_output(),
            default_reply(),
            "{query:?} one byte per feed"
        );
    }
}

#[test]
fn split_query_between_other_output_is_answered_once() {
    let stream = b"text \x1b[1mbold\x1b[>0q more \x1b[>q end";
    for split in 1..stream.len() {
        let mut view = view();
        view.feed(&stream[..split]);
        view.feed(&stream[split..]);
        let output = view.take_output();
        assert_eq!(count(&output, &default_reply()), 2, "split at {split}");
        assert_eq!(output.len(), 2 * default_reply().len(), "split at {split}");
    }
}

#[test]
fn near_misses_split_anywhere_get_no_reply() {
    for stream in [&b"\x1b[>1q"[..], b"\x1b[>0c", b"\x1b[>0;1q"] {
        assert_replies(stream, 0);
    }
}

#[test]
fn a_near_miss_then_a_query_in_a_later_read_gets_one_reply() {
    let cases: [&[&[u8]]; 3] = [
        &[b"\x1b[>", b"1q", b"\x1b[>0q"],
        &[b"\x1b[>0", b";1q", b"\x1b[>q"],
        &[b"\x1b[>0", b"c", b"\x1b[>0q"],
    ];
    for feeds in cases {
        let (output, queries) = replies_and_queries(feeds);
        assert_eq!(queries, 1, "the parser reads {feeds:?} otherwise");
        assert_eq!(count(&output, b"\x1bP>|"), 1, "{feeds:?}");
        assert_eq!(count(&output, &default_reply()), 1, "{feeds:?}");
        assert_replies(&feeds.concat(), 1);
    }
}

#[test]
fn a_query_after_a_cut_short_sequence_gets_one_reply() {
    // An escape inside an unfinished sequence starts a new one, so the parser reads the query.
    for stream in [&b"\x1b\x1b[>q"[..], b"\x1b[\x1b[>q", b"\x1b[>0\x1b[>0q"] {
        assert_replies(stream, 1);
    }
}

#[test]
fn a_query_after_a_multibyte_character_gets_one_reply() {
    // Cuts fall inside the characters too.
    for stream in [
        "\u{e9}\x1b[>0q",
        "\u{20ac}\x1b[>q",
        "a \u{e9}\u{20ac} b\x1b[>0q c",
    ] {
        assert_replies(stream.as_bytes(), 1);
    }
}

#[test]
fn identity_is_set_by_the_program() {
    let mut view = view();
    view.set_identity("paw 9.9.9");
    view.feed(QUERY);
    assert_eq!(view.take_output(), reply("paw 9.9.9"));
}

#[test]
fn identity_drops_control_bytes() {
    let mut view = view();
    view.set_identity("snow\x1b\\flake\x07 1\n.2\u{e9}");
    view.feed(QUERY);
    let output = view.take_output();
    assert_eq!(output, reply("snow\\flake 1.2"));
    assert!(output.starts_with(b"\x1bP>|"));
    assert!(output.ends_with(b"\x1b\\"));
    assert_eq!(count(&output, b"\x1b\\"), 1);
    let inside = &output[2..output.len() - 2];
    assert!(
        inside.iter().all(|byte| (0x20..=0x7e).contains(byte)),
        "{inside:?}"
    );
}

#[test]
fn identity_that_ends_up_empty_falls_back_to_the_default() {
    let mut view = view();
    view.set_identity("\x1b\x07\n");
    view.feed(QUERY);
    assert_eq!(view.take_output(), default_reply());
    view.set_identity("");
    view.feed(QUERY);
    assert_eq!(view.take_output(), default_reply());
}

#[test]
fn xtversion_then_attributes_reply_in_order() {
    let mut view = view();
    view.feed(b"\x1b[>0q\x1b[c");
    let mut expected = default_reply();
    expected.extend_from_slice(b"\x1b[?6c");
    assert_eq!(view.take_output(), expected);

    view.feed(b"\x1b[c\x1b[>q\x1b[6n");
    let mut expected = b"\x1b[?6c".to_vec();
    expected.extend_from_slice(&default_reply());
    expected.extend_from_slice(b"\x1b[1;1R");
    assert_eq!(view.take_output(), expected);
}

#[test]
fn two_queries_two_replies() {
    let mut view = view();
    view.feed(b"\x1b[>0q\x1b[>q");
    assert_eq!(
        view.take_output(),
        [default_reply(), default_reply()].concat()
    );
}

#[test]
fn plain_text_with_the_same_letters_gets_no_reply() {
    let mut view = view();
    view.feed(b"[>0q [>q >0q");
    assert_eq!(view.take_output(), b"");
    assert!(view.screen().row_text(0).starts_with("[>0q [>q >0q"));
}

#[test]
fn query_after_an_unfinished_title_is_still_a_query() {
    // An escape ends any string sequence, so these bytes are a real query to the parser too.
    let mut view = view();
    view.feed(b"\x1b]0;title\x1b[>0q");
    assert_eq!(view.take_output(), default_reply());
}

#[test]
fn xtversion_inside_a_synchronized_update_is_answered_at_once() {
    let mut view = view();
    view.feed(b"\x1b[?2026hhello\x1b[>0q");
    assert_eq!(view.take_output(), default_reply());
    view.feed(b"\x1b[?2026l");
    assert_eq!(view.take_output(), b"");
}
