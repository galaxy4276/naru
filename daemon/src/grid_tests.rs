use super::*;

fn rows(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|line| line.to_string()).collect()
}

#[test]
fn reads_the_last_lines_joining_wrapped_rows() {
    let mut screen = Screen::new(10, 5);
    screen.feed(b"one\r\n0123456789abc\r\nthree\r\n\r\n");
    assert_eq!(screen.last_lines(10), "one\n0123456789abc\nthree");
    assert_eq!(screen.last_lines(2), "0123456789abc\nthree");
}

#[test]
fn keeps_lines_scrolled_off_the_screen() {
    let mut screen = Screen::new(20, 3);
    for line in 0..10 {
        screen.feed(format!("line {line}\r\n").as_bytes());
    }
    assert_eq!(screen.last_lines(4), "line 6\nline 7\nline 8\nline 9");
}

#[test]
fn reads_only_the_alternate_screen() {
    let mut screen = Screen::new(20, 3);
    screen.feed(b"history\r\n");
    screen.feed(b"\x1b[?1049h\x1b[Hfull screen");
    assert_eq!(screen.last_lines(10), "full screen");
    screen.feed(b"\x1b[?1049l");
    assert!(screen.last_lines(10).starts_with("history"));
}

#[test]
fn notices_bracketed_paste() {
    let mut screen = Screen::new(20, 3);
    assert!(!screen.bracketed_paste());
    screen.feed(b"\x1b[?2004h");
    assert!(screen.bracketed_paste());
}

#[test]
fn reads_what_the_input_line_holds() {
    assert_eq!(
        read_draft(&rows(&["output", "╭───╮", "│ >   │"])),
        Draft::Empty
    );
    assert_eq!(read_draft(&rows(&["❯ fix the bug"])), Draft::Other);
    assert_eq!(
        read_draft(&rows(&["› [Naru letter from a1 \"x\""])),
        Draft::Letter
    );
    assert_eq!(
        read_draft(&rows(&["> [Pasted text #1 +12 lines]"])),
        Draft::Letter
    );
    assert_eq!(
        read_draft(&rows(&["Do you want to proceed?", "1. Yes"])),
        Draft::Unseen
    );
    // A quoted reply in the output is not an input line.
    assert_eq!(read_draft(&rows(&[">quoted", "❯ "])), Draft::Empty);
}

#[test]
fn blanks_a_dim_placeholder() {
    let mut screen = Screen::new(30, 4);
    screen.feed(b"\xe2\x9d\xaf \x1b[2mTry \"fix lint\"\x1b[0m\r\n");
    assert_eq!(screen.draft(), Draft::Empty);
    screen.feed(b"\xe2\x9d\xaf hello\r\n");
    assert_eq!(screen.draft(), Draft::Other);
}
