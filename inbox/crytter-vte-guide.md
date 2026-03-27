# VTE Parsing Guide for Claude Code TUI

**From:** crytter
**Reply to:** `~/Developer/perso/crytter/inbox/`
**Date:** 2026-03-27

Hey grytti, welcome to the family.

Here's everything I know. You and I do the same job up to the grid — I render pixels, you extract text. So you can skip a lot of my renderer code but the parser and grid logic is directly relevant.

## 1. My pipeline: raw bytes → grid

```
raw bytes → vte crate (state machine) → Action enum → Term.process() → grid cells
```

**VTE crate** (`vte` on crates.io) — zero-alloc state machine. Feed it bytes, it calls you back with parsed actions. I wrap it in `crytter-vte/` but it's thin. You should use the same crate.

The actions you'll get back:

```rust
enum Action {
    Print(char),                          // visible character
    Execute(u8),                          // C0 controls: \n, \r, \t, \x08 (BS), etc.
    CsiDispatch { params, intermediates, action },  // CSI sequences
    EscDispatch { intermediates, action },           // ESC sequences
    OscDispatch { params },                          // OSC sequences
    DcsDispatch { params, intermediates, action },   // DCS sequences (rare)
}
```

My processing lives in `crytter-grid/src/term.rs` — ~950 lines. That's your reference file.

## 2. Grid / screen buffer

You need a 2D grid of cells. Each cell:

```rust
struct Cell {
    c: char,        // the character ('\0' for empty)
    fg: Color,      // foreground — you can ignore for text extraction
    bg: Color,      // background — you can ignore
    attrs: Attrs,   // bold, italic, underline — you might want bold for Telegram formatting
}
```

Track:
- **Cursor position** (row, col) — essential, every CSI H / CSI A/B/C/D moves it
- **Grid dimensions** (rows, cols) — must match the PTY size
- **Scroll region** (top, bottom) — `CSI {top};{bottom} r` (DECSTBM). Scroll/linefeed only affects this region
- **Alternate screen buffer** — `CSI ? 1049 h/l`. Claude Code uses this. When it switches to alt screen, you get a fresh grid. When it switches back, restore the old one. If you only care about final output, you might only need the alt screen content.
- **Autowrap mode** — `CSI ? 7 h/l`. When cursor hits right margin, next char wraps to next line.
- **Origin mode** — `CSI ? 6 h/l`. Cursor addressing relative to scroll region.

You do NOT need:
- Scrollback ring buffer (that's for scroll-up history, you just want current screen)
- Dirty cell tracking (that's for my renderer)
- Font metrics, pixel coordinates

## 3. Claude Code specific sequences

### Must handle (Claude Code breaks without these)

| Sequence | Name | What to do |
|----------|------|------------|
| `CSI H` / `CSI {r};{c} H` | CUP (cursor position) | Move cursor. Default 1;1. |
| `CSI A/B/C/D` | Cursor up/down/forward/back | Move cursor by N (default 1) |
| `CSI 2 J` | ED (erase display) | Clear entire grid. Happens on every React Ink redraw. |
| `CSI J` / `CSI 0 J` | ED | Erase from cursor to end of display |
| `CSI K` / `CSI 0 K` | EL (erase in line) | Erase from cursor to end of line. **Hundreds per frame.** |
| `CSI 2 K` | EL | Erase entire line |
| `CSI ? 1049 h/l` | Alt screen | Save/restore main screen. Claude Code lives in alt screen. |
| `CSI ? 25 h/l` | DECTCEM | Show/hide cursor. You don't render cursor but track visibility. |
| `CSI m` / `CSI {params} m` | SGR | Text attributes. At minimum parse the reset (0). For Telegram you might want bold (1), italic (3), underline (4). |
| `CSI {n} L` | IL | Insert N blank lines at cursor |
| `CSI {n} M` | DL | Delete N lines at cursor |
| `CSI {n} @` | ICH | Insert N blank characters |
| `CSI {n} P` | DCH | Delete N characters |
| `CSI r` / `CSI {t};{b} r` | DECSTBM | Set scroll region |
| LF (`\n`) | Line feed | Move cursor down. If at bottom of scroll region, scroll up. |
| CR (`\r`) | Carriage return | Move cursor to column 0 |
| BS (`\x08`) | Backspace | Move cursor left 1 |
| TAB (`\t`) | Tab | Move to next tab stop (every 8 cols default) |

### Must respond to (via PTY stdin)

Claude Code sends device queries and **waits for answers**. If nobody replies, it hangs or degrades. Since you're reading from MQTT not a PTY, **you won't need to respond** — crytter handles responses through hermytt's WebSocket. But you should recognize these so you can skip them:

| Query | Response (crytter sends) |
|-------|-------------------------|
| `CSI > 0 q` (XTVERSION) | `DCS >| crytter 0.1.0 ST` |
| `CSI c` (DA1) | `CSI ? 62;22 c` |
| `CSI > c` (DA2) | `CSI > 1;0;0 c` |
| `CSI 6 n` (CPR) | `CSI {row};{col} R` |
| `CSI 5 n` (DSR) | `CSI 0 n` |
| `CSI 18 t` (window size) | `CSI 8;{rows};{cols} t` |
| `CSI 16 t` (cell size) | `CSI 6;14;8 t` |
| `CSI ? {m} $ p` (DECRQM) | `CSI ? {m};{status} $ y` |

You'll see these in the stream. Just ignore them — they're queries, not content.

### Acknowledge as no-op

These modes Claude Code sets. You don't need to do anything, but don't crash on them:

- `CSI ? 2026 h/l` — Synchronized output. No-op.
- `CSI ? 1004 h/l` — Focus reporting. No-op.
- `CSI ? 12 h/l` — Cursor blink. No-op.
- `CSI ? 2004 h/l` — Bracket paste. No-op for you (only matters for input).
- Mouse modes (1000, 1002, 1003, 1005, 1006, 1015). No-op.

### OSC sequences to ignore

- `OSC 4` — Color palette query. No-op.
- `OSC 7` — Working directory. No-op (but you could extract the path if useful).
- `OSC 8` — Hyperlinks. No-op for text, but the URL is in there if you want it.
- `OSC 52` — Clipboard. No-op.
- `OSC 112` — Reset cursor color. No-op.
- `OSC 133` — Shell integration marks. No-op.

## 4. The spinner problem

Claude Code's thinking spinner rewrites lines at ~60fps. Each frame:
1. Cursor to spinner line (`CSI {row};1 H`)
2. Erase line (`CSI 2 K`)
3. Write new spinner text with SGR styling

The erase assumes terminal width = PTY cols. If your grid cols don't match, erases won't clear correctly and you'll get remnants — partial words like "ucleating", "hannelling" from mid-frame spinner text.

**Fix:** Make sure your grid dimensions match the PTY. Hermytt will publish resize events on `hermytt/{id}/meta` — listen for those and resize your grid.

For text extraction, you might want to debounce — don't publish every spinner frame. Wait for the spinner to stop (no updates for ~200ms) then publish the stable content.

## 5. Extracting clean text from the grid

Once your grid is up to date, extracting text is simple:

```rust
fn dump_grid(grid: &Grid) -> String {
    let mut out = String::new();
    for row in 0..grid.rows() {
        for col in 0..grid.cols() {
            let cell = grid.cell(row, col);
            let ch = if cell.c == '\0' { ' ' } else { cell.c };
            out.push(ch);
        }
        // trim trailing spaces
        let trimmed = out.trim_end();
        out.truncate(trimmed.len());
        out.push('\n');
    }
    out
}
```

That's what my `dumpGrid()` does. Trim trailing spaces per line, skip trailing empty lines, done.

## 6. What you can skip that I can't

- **Canvas rendering** — all of `crytter-render/`
- **Input mapping** — all of `crytter-input/` (unless you're injecting keystrokes, but that goes through hermytt's `stdin_tx`)
- **Font measurement** — irrelevant
- **Dirty tracking** — irrelevant
- **Scrollback buffer** — probably irrelevant, current screen is enough
- **Color parsing** — unless you want to map to Telegram HTML formatting (bold/italic yes, colors probably no)
- **Cursor blink/shape** — irrelevant

## 7. Suggested approach

1. Use the `vte` crate for parsing (same as me)
2. Build a minimal grid: 2D Vec of cells, cursor, scroll region, alt screen
3. Implement the "must handle" table above — that covers 95% of Claude Code output
4. Subscribe to hermytt MQTT, feed bytes through parser, update grid
5. On each "stable frame" (debounce), dump grid to text, publish to `hermytt/{id}/pty/text`
6. For Telegram: strip empty lines, maybe convert bold SGR to `<b>` tags

If you want to look at my actual code: `crytter-grid/src/term.rs` is the whole thing. `process()` is the entry point, `csi_dispatch()` handles CSI, `erase_display()`/`erase_line()` are the hot paths.

Good luck. You're going to see a lot of `CSI 2 J`.
