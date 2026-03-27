use vte::{Params, Parser, Perform};

use crate::grid::Grid;

/// VTE performer that writes parsed output into a Grid.
pub struct GridPerformer {
    pub grid: Grid,
}

impl GridPerformer {
    pub fn new(grid: Grid) -> Self {
        Self { grid }
    }

    /// Feed raw PTY bytes through the VTE parser.
    pub fn feed(parser: &mut Parser, performer: &mut GridPerformer, bytes: &[u8]) {
        for &byte in bytes {
            parser.advance(performer, byte);
        }
    }

    fn param(params: &Params, idx: usize, default: u16) -> u16 {
        params
            .iter()
            .nth(idx)
            .and_then(|p| p.first().copied())
            .filter(|&v| v != 0)
            .unwrap_or(default)
    }
}

impl Perform for GridPerformer {
    fn print(&mut self, c: char) {
        self.grid.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x0A => self.grid.line_feed(),
            0x0D => self.grid.carriage_return(),
            0x08 => self.grid.backspace(),
            0x09 => self.grid.tab(),
            0x07 => {} // BEL
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        // Check for DEC private modes (intermediates contains '?')
        let is_private = intermediates.contains(&b'?');

        match action {
            // Cursor movement
            'A' => self.grid.move_up(Self::param(params, 0, 1) as usize),
            'B' => self.grid.move_down(Self::param(params, 0, 1) as usize),
            'C' => self.grid.move_forward(Self::param(params, 0, 1) as usize),
            'D' if !is_private => self.grid.move_backward(Self::param(params, 0, 1) as usize),
            'E' => {
                let n = Self::param(params, 0, 1) as usize;
                self.grid.move_down(n);
                self.grid.carriage_return();
            }
            'F' => {
                let n = Self::param(params, 0, 1) as usize;
                self.grid.move_up(n);
                self.grid.carriage_return();
            }
            'G' => {
                let col = Self::param(params, 0, 1) as usize;
                self.grid.cursor_col = (col - 1).min(self.grid.cols - 1);
                self.grid.pending_wrap = false;
            }
            'H' | 'f' => {
                let row = Self::param(params, 0, 1) as usize;
                let col = Self::param(params, 1, 1) as usize;
                self.grid.set_cursor(row - 1, col - 1);
            }
            'J' => {
                let mode = Self::param(params, 0, 0);
                self.grid.erase_display(mode);
            }
            'K' => {
                let mode = Self::param(params, 0, 0);
                self.grid.erase_line(mode);
            }
            'L' => {
                let n = Self::param(params, 0, 1) as usize;
                self.grid.insert_lines(n);
            }
            'M' => {
                let n = Self::param(params, 0, 1) as usize;
                self.grid.delete_lines(n);
            }
            '@' => {
                let n = Self::param(params, 0, 1) as usize;
                self.grid.insert_chars(n);
            }
            'P' => {
                let n = Self::param(params, 0, 1) as usize;
                self.grid.delete_chars(n);
            }
            'S' => {
                let n = Self::param(params, 0, 1) as usize;
                for _ in 0..n {
                    self.grid.line_feed();
                }
            }
            'd' => {
                // VPA — vertical position absolute
                let row = Self::param(params, 0, 1) as usize;
                self.grid.cursor_row = (row - 1).min(self.grid.rows - 1);
                self.grid.pending_wrap = false;
            }
            'r' => {
                // DECSTBM — set scroll region
                let top = Self::param(params, 0, 1) as usize;
                let bottom = Self::param(params, 1, 0) as usize;
                if bottom == 0 {
                    self.grid.reset_scroll_region();
                } else {
                    self.grid.set_scroll_region(top - 1, bottom - 1);
                }
            }
            's' if !is_private => self.grid.save_cursor(),
            'u' => self.grid.restore_cursor(),
            'm' => {
                // SGR — select graphic rendition
                if params.len() == 0 {
                    self.grid.reset_attrs();
                } else {
                    for param in params.iter() {
                        match param.first().copied().unwrap_or(0) {
                            0 => self.grid.reset_attrs(),
                            1 => self.grid.cur_bold = true,
                            3 => self.grid.cur_italic = true,
                            4 => self.grid.cur_underline = true,
                            22 => self.grid.cur_bold = false,
                            23 => self.grid.cur_italic = false,
                            24 => self.grid.cur_underline = false,
                            _ => {} // Colors and other attrs — ignore for text
                        }
                    }
                }
            }
            'h' | 'l' => {
                if is_private {
                    let mode = Self::param(params, 0, 0);
                    let enable = action == 'h';
                    match mode {
                        1049 | 1047 => {
                            if enable {
                                self.grid.enter_alternate_screen();
                            } else {
                                self.grid.exit_alternate_screen();
                            }
                        }
                        7 => self.grid.set_autowrap(enable),
                        // 25 = cursor visibility, 2004 = bracketed paste,
                        // 2026 = synchronized output, 1004 = focus reporting,
                        // 12 = cursor blink, 1000/1002/1003/1005/1006/1015 = mouse modes
                        // All no-ops for text extraction
                        _ => {}
                    }
                }
            }
            // Device queries — ignore (crytter handles responses)
            'c' | 'n' | 'q' => {}
            't' => {
                // Window ops / queries — ignore
            }
            _ => {
                tracing::trace!("unhandled CSI: {:?} {:?}", action, params);
            }
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        match byte {
            b'7' => self.grid.save_cursor(),
            b'8' => self.grid.restore_cursor(),
            b'M' => self.grid.reverse_index(),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // OSC (title, hyperlinks, clipboard, shell integration) — ignore
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
}
