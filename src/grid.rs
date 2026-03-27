/// Virtual terminal grid — tracks cursor and screen content.
/// Enough fidelity to reconstruct readable text from Claude Code's TUI output.

const DEFAULT_COLS: usize = 120;
const DEFAULT_ROWS: usize = 40;

#[derive(Debug, Clone)]
pub struct Cell {
    pub c: char,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            bold: false,
            italic: false,
            underline: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Grid {
    cells: Vec<Vec<Cell>>,
    pub cols: usize,
    pub rows: usize,
    pub cursor_row: usize,
    pub cursor_col: usize,
    /// Scroll region (top, bottom) — inclusive, 0-indexed
    scroll_top: usize,
    scroll_bottom: usize,
    /// Whether we're in alternate screen buffer
    pub alternate_screen: bool,
    /// Saved main screen (when entering alt screen)
    saved_main: Option<SavedScreen>,
    /// Saved cursor position (DECSC/DECRC)
    saved_cursor: Option<(usize, usize)>,
    /// Autowrap mode (CSI ? 7 h/l)
    autowrap: bool,
    /// Pending wrap — cursor is past the right margin, next print wraps
    pub pending_wrap: bool,
    /// Current SGR attributes for new characters
    pub cur_bold: bool,
    pub cur_italic: bool,
    pub cur_underline: bool,
}

#[derive(Debug, Clone)]
struct SavedScreen {
    cells: Vec<Vec<Cell>>,
    cursor_row: usize,
    cursor_col: usize,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cells: vec![vec![Cell::default(); cols]; rows],
            cols,
            rows,
            cursor_row: 0,
            cursor_col: 0,
            scroll_top: 0,
            scroll_bottom: rows - 1,
            alternate_screen: false,
            saved_main: None,
            saved_cursor: None,
            autowrap: true,
            pending_wrap: false,
            cur_bold: false,
            cur_italic: false,
            cur_underline: false,
        }
    }

    pub fn default() -> Self {
        Self::new(DEFAULT_COLS, DEFAULT_ROWS)
    }

    pub fn put_char(&mut self, c: char) {
        if self.pending_wrap && self.autowrap {
            self.cursor_col = 0;
            self.line_feed();
            self.pending_wrap = false;
        }
        if self.cursor_row < self.rows && self.cursor_col < self.cols {
            self.cells[self.cursor_row][self.cursor_col] = Cell {
                c,
                bold: self.cur_bold,
                italic: self.cur_italic,
                underline: self.cur_underline,
            };
            if self.cursor_col + 1 >= self.cols {
                // At right margin — set pending wrap
                if self.autowrap {
                    self.pending_wrap = true;
                }
            } else {
                self.cursor_col += 1;
            }
        }
    }

    pub fn line_feed(&mut self) {
        self.pending_wrap = false;
        if self.cursor_row == self.scroll_bottom {
            self.scroll_up_region(1);
        } else if self.cursor_row < self.rows - 1 {
            self.cursor_row += 1;
        }
    }

    pub fn carriage_return(&mut self) {
        self.cursor_col = 0;
        self.pending_wrap = false;
    }

    pub fn backspace(&mut self) {
        self.pending_wrap = false;
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        }
    }

    pub fn tab(&mut self) {
        self.pending_wrap = false;
        let next_tab = (self.cursor_col / 8 + 1) * 8;
        self.cursor_col = next_tab.min(self.cols - 1);
    }

    /// Scroll the scroll region up by n lines
    fn scroll_up_region(&mut self, n: usize) {
        for _ in 0..n {
            if self.scroll_top < self.scroll_bottom {
                self.cells.remove(self.scroll_top);
                self.cells
                    .insert(self.scroll_bottom, vec![Cell::default(); self.cols]);
            }
        }
    }

    /// Scroll the scroll region down by n lines
    fn scroll_down_region(&mut self, n: usize) {
        for _ in 0..n {
            if self.scroll_top < self.scroll_bottom {
                self.cells.remove(self.scroll_bottom);
                self.cells
                    .insert(self.scroll_top, vec![Cell::default(); self.cols]);
            }
        }
    }

    /// Reverse index — move cursor up, scroll down if at top of scroll region
    pub fn reverse_index(&mut self) {
        if self.cursor_row == self.scroll_top {
            self.scroll_down_region(1);
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
        }
    }

    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.cursor_row = row.min(self.rows - 1);
        self.cursor_col = col.min(self.cols - 1);
        self.pending_wrap = false;
    }

    pub fn save_cursor(&mut self) {
        self.saved_cursor = Some((self.cursor_row, self.cursor_col));
    }

    pub fn restore_cursor(&mut self) {
        if let Some((r, c)) = self.saved_cursor {
            self.cursor_row = r;
            self.cursor_col = c;
            self.pending_wrap = false;
        }
    }

    pub fn move_up(&mut self, n: usize) {
        self.cursor_row = self.cursor_row.saturating_sub(n);
        self.pending_wrap = false;
    }

    pub fn move_down(&mut self, n: usize) {
        self.cursor_row = (self.cursor_row + n).min(self.rows - 1);
        self.pending_wrap = false;
    }

    pub fn move_forward(&mut self, n: usize) {
        self.cursor_col = (self.cursor_col + n).min(self.cols - 1);
        self.pending_wrap = false;
    }

    pub fn move_backward(&mut self, n: usize) {
        self.cursor_col = self.cursor_col.saturating_sub(n);
        self.pending_wrap = false;
    }

    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let top = top.min(self.rows - 1);
        let bottom = bottom.min(self.rows - 1);
        if top < bottom {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
            // Cursor moves to home after setting scroll region
            self.cursor_row = 0;
            self.cursor_col = 0;
            self.pending_wrap = false;
        }
    }

    pub fn reset_scroll_region(&mut self) {
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
    }

    pub fn set_autowrap(&mut self, on: bool) {
        self.autowrap = on;
    }

    /// Erase in display: 0 = cursor to end, 1 = start to cursor, 2/3 = entire
    pub fn erase_display(&mut self, mode: u16) {
        match mode {
            0 => {
                for col in self.cursor_col..self.cols {
                    self.cells[self.cursor_row][col] = Cell::default();
                }
                for row in (self.cursor_row + 1)..self.rows {
                    self.cells[row] = vec![Cell::default(); self.cols];
                }
            }
            1 => {
                for row in 0..self.cursor_row {
                    self.cells[row] = vec![Cell::default(); self.cols];
                }
                for col in 0..=self.cursor_col.min(self.cols - 1) {
                    self.cells[self.cursor_row][col] = Cell::default();
                }
            }
            2 | 3 => {
                for row in 0..self.rows {
                    self.cells[row] = vec![Cell::default(); self.cols];
                }
            }
            _ => {}
        }
    }

    /// Erase in line: 0 = cursor to end, 1 = start to cursor, 2 = entire line
    pub fn erase_line(&mut self, mode: u16) {
        let row = self.cursor_row;
        match mode {
            0 => {
                for col in self.cursor_col..self.cols {
                    self.cells[row][col] = Cell::default();
                }
            }
            1 => {
                for col in 0..=self.cursor_col.min(self.cols - 1) {
                    self.cells[row][col] = Cell::default();
                }
            }
            2 => {
                self.cells[row] = vec![Cell::default(); self.cols];
            }
            _ => {}
        }
    }

    /// Insert n blank lines at cursor row, pushing lines down within scroll region
    pub fn insert_lines(&mut self, n: usize) {
        for _ in 0..n {
            if self.cursor_row <= self.scroll_bottom {
                if self.scroll_bottom < self.cells.len() {
                    self.cells.remove(self.scroll_bottom);
                }
                self.cells
                    .insert(self.cursor_row, vec![Cell::default(); self.cols]);
            }
        }
    }

    /// Delete n lines at cursor row, pulling lines up within scroll region
    pub fn delete_lines(&mut self, n: usize) {
        for _ in 0..n {
            if self.cursor_row <= self.scroll_bottom {
                self.cells.remove(self.cursor_row);
                self.cells
                    .insert(self.scroll_bottom, vec![Cell::default(); self.cols]);
            }
        }
    }

    /// Insert n blank characters at cursor, shifting right
    pub fn insert_chars(&mut self, n: usize) {
        let row = self.cursor_row;
        for _ in 0..n {
            if self.cursor_col < self.cols {
                self.cells[row].pop();
                self.cells[row]
                    .insert(self.cursor_col, Cell::default());
            }
        }
        // Ensure row length stays correct
        self.cells[row].resize(self.cols, Cell::default());
    }

    /// Delete n characters at cursor, shifting left
    pub fn delete_chars(&mut self, n: usize) {
        let row = self.cursor_row;
        for _ in 0..n {
            if self.cursor_col < self.cells[row].len() {
                self.cells[row].remove(self.cursor_col);
                self.cells[row].push(Cell::default());
            }
        }
    }

    pub fn enter_alternate_screen(&mut self) {
        self.saved_main = Some(SavedScreen {
            cells: self.cells.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
        });
        self.alternate_screen = true;
        self.erase_display(2);
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.pending_wrap = false;
    }

    pub fn exit_alternate_screen(&mut self) {
        self.alternate_screen = false;
        if let Some(saved) = self.saved_main.take() {
            self.cells = saved.cells;
            self.cursor_row = saved.cursor_row;
            self.cursor_col = saved.cursor_col;
        }
        self.pending_wrap = false;
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let mut new_cells = vec![vec![Cell::default(); cols]; rows];
        for r in 0..rows.min(self.rows) {
            for c in 0..cols.min(self.cols) {
                new_cells[r][c] = self.cells[r][c].clone();
            }
        }
        self.cells = new_cells;
        self.cols = cols;
        self.rows = rows;
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.cursor_row = self.cursor_row.min(rows - 1);
        self.cursor_col = self.cursor_col.min(cols - 1);
        self.pending_wrap = false;
    }

    /// SGR reset
    pub fn reset_attrs(&mut self) {
        self.cur_bold = false;
        self.cur_italic = false;
        self.cur_underline = false;
    }

    /// Snapshot the entire screen as text, one line per row, trailing whitespace trimmed.
    pub fn snapshot(&self) -> String {
        self.cells
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| if cell.c == '\0' { ' ' } else { cell.c })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_string()
    }
}
