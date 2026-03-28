use vte::Parser;

// We need to access grytti internals
// Since these are integration tests, we test through the public API

#[path = "../src/grid.rs"]
mod grid;
#[path = "../src/parser.rs"]
mod parser;

use grid::Grid;
use parser::GridPerformer;

fn feed(grid_cols: usize, grid_rows: usize, input: &[u8]) -> String {
    let grid = Grid::new(grid_cols, grid_rows);
    let mut parser = Parser::new();
    let mut performer = GridPerformer::new(grid);
    GridPerformer::feed(&mut parser, &mut performer, input);
    performer.grid.snapshot()
}

// === Basic text output ===

#[test]
fn plain_text() {
    let snap = feed(40, 5, b"Hello, world!");
    assert_eq!(snap, "Hello, world!");
}

#[test]
fn text_with_newline() {
    let snap = feed(40, 5, b"line1\r\nline2\r\nline3");
    assert_eq!(snap, "line1\nline2\nline3");
}

#[test]
fn carriage_return_overwrites() {
    let snap = feed(40, 5, b"AAAA\rBB");
    assert_eq!(snap, "BBAA");
}

// === Cursor movement ===

#[test]
fn cursor_position_cup() {
    // CSI 2;5 H = move to row 2, col 5
    let snap = feed(20, 5, b"\x1b[2;5Hhello");
    let lines: Vec<&str> = snap.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1], "    hello");
}

#[test]
fn cursor_up_down() {
    let snap = feed(20, 5, b"line1\r\nline2\x1b[Aup");
    let lines: Vec<&str> = snap.lines().collect();
    assert_eq!(lines[0], "line1up");
}

#[test]
fn cursor_forward_back() {
    // Write "ABCD", back 2, write "XX" → "ABXX" wait no "AXXD"
    let snap = feed(20, 5, b"ABCD\x1b[2DXX");
    assert_eq!(snap, "ABXX");
}

#[test]
fn cursor_horizontal_absolute() {
    // CSI 5 G = move to column 5
    let snap = feed(20, 5, b"0123456789\x1b[5GX");
    assert_eq!(snap, "0123X56789");
}

// === Erase operations ===

#[test]
fn erase_display_full() {
    // CSI 2 J = erase entire display
    let snap = feed(20, 5, b"hello\r\nworld\x1b[2J");
    assert_eq!(snap, "");
}

#[test]
fn erase_line_to_end() {
    // CSI 0 K = erase from cursor to end of line
    let snap = feed(20, 5, b"hello world\x1b[6G\x1b[K");
    assert_eq!(snap, "hello");
}

#[test]
fn erase_entire_line() {
    // CSI 2 K = erase entire line
    let snap = feed(20, 5, b"first\r\nsecond\x1b[2K");
    let lines: Vec<&str> = snap.lines().collect();
    assert_eq!(lines[0], "first");
    // Second line erased — snapshot trims empty trailing lines
}

// === Scroll region ===

#[test]
fn scroll_region_linefeed() {
    // Set scroll region to rows 1-3, fill them, then LF at bottom scrolls within region
    let mut input = Vec::new();
    input.extend_from_slice(b"\x1b[1;3r");     // set scroll region rows 1-3
    input.extend_from_slice(b"\x1b[1;1H");     // cursor to 1,1
    input.extend_from_slice(b"AAA\r\nBBB\r\nCCC"); // fill 3 rows
    input.extend_from_slice(b"\r\nDDD");        // LF at bottom of region → scroll
    let snap = feed(10, 5, &input);
    let lines: Vec<&str> = snap.lines().collect();
    // After scroll: BBB, CCC, DDD (AAA scrolled off)
    assert_eq!(lines[0], "BBB");
    assert_eq!(lines[1], "CCC");
    assert_eq!(lines[2], "DDD");
}

// === Alternate screen buffer ===

#[test]
fn alt_screen_save_restore() {
    let grid = Grid::new(20, 5);
    let mut parser = Parser::new();
    let mut performer = GridPerformer::new(grid);

    // Write on main screen
    GridPerformer::feed(&mut parser, &mut performer, b"main screen");
    assert!(performer.grid.snapshot().contains("main screen"));

    // Enter alt screen
    GridPerformer::feed(&mut parser, &mut performer, b"\x1b[?1049h");
    assert!(performer.grid.alternate_screen);
    assert_eq!(performer.grid.snapshot(), ""); // alt screen is blank

    // Write on alt screen
    GridPerformer::feed(&mut parser, &mut performer, b"alt content");
    assert!(performer.grid.snapshot().contains("alt content"));

    // Exit alt screen — main screen restored
    GridPerformer::feed(&mut parser, &mut performer, b"\x1b[?1049l");
    assert!(!performer.grid.alternate_screen);
    assert!(performer.grid.snapshot().contains("main screen"));
}

// === Autowrap ===

#[test]
fn autowrap_at_right_margin() {
    // Grid is 10 cols wide, write 15 chars → wraps to next line
    let snap = feed(10, 5, b"1234567890ABCDE");
    let lines: Vec<&str> = snap.lines().collect();
    assert_eq!(lines[0], "1234567890");
    assert_eq!(lines[1], "ABCDE");
}

// === Insert/Delete lines and characters ===

#[test]
fn insert_lines() {
    // CSI L = insert blank line at cursor
    let snap = feed(20, 5, b"AAA\r\nBBB\r\nCCC\x1b[2;1H\x1b[L");
    let lines: Vec<&str> = snap.lines().collect();
    assert_eq!(lines[0], "AAA");
    assert_eq!(lines[1], ""); // inserted blank line
    assert_eq!(lines[2], "BBB");
}

#[test]
fn delete_lines() {
    // CSI M = delete line at cursor
    let snap = feed(20, 5, b"AAA\r\nBBB\r\nCCC\x1b[2;1H\x1b[M");
    let lines: Vec<&str> = snap.lines().collect();
    assert_eq!(lines[0], "AAA");
    assert_eq!(lines[1], "CCC");
}

#[test]
fn insert_chars() {
    // CSI @ = insert blank characters
    let snap = feed(20, 5, b"ABCDEF\x1b[1;3H\x1b[2@");
    assert_eq!(snap, "AB  CDEF");
}

#[test]
fn delete_chars() {
    // CSI P = delete characters
    let snap = feed(20, 5, b"ABCDEF\x1b[1;3H\x1b[2P");
    assert_eq!(snap, "ABEF");
}

// === Tab stops ===

#[test]
fn tab_stops() {
    let snap = feed(40, 5, b"A\tB\tC");
    assert_eq!(snap, "A       B       C");
}

// === Backspace ===

#[test]
fn backspace() {
    let snap = feed(20, 5, b"ABC\x08X");
    assert_eq!(snap, "ABX");
}

// === Reverse index ===

#[test]
fn reverse_index() {
    // ESC M at top of screen → scroll down
    let snap = feed(20, 5, b"first\r\nsecond\x1b[1;1H\x1bMinserted");
    let lines: Vec<&str> = snap.lines().collect();
    assert_eq!(lines[0], "inserted");
    assert_eq!(lines[1], "first");
    assert_eq!(lines[2], "second");
}

// === SGR tracking ===

#[test]
fn sgr_bold_tracked() {
    let grid = Grid::new(20, 5);
    let mut parser = Parser::new();
    let mut performer = GridPerformer::new(grid);

    GridPerformer::feed(&mut parser, &mut performer, b"\x1b[1mBOLD\x1b[0m");
    assert!(!performer.grid.cur_bold); // reset after SGR 0
}

// === Resize ===

#[test]
fn resize_preserves_content() {
    let mut grid = Grid::new(20, 5);
    let mut parser = Parser::new();
    let mut performer = GridPerformer::new(grid);

    GridPerformer::feed(&mut parser, &mut performer, b"hello");
    performer.grid.resize(40, 10);
    assert!(performer.grid.snapshot().contains("hello"));
    assert_eq!(performer.grid.cols, 40);
    assert_eq!(performer.grid.rows, 10);
}
