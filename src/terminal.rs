use crate::theme::Theme;
use ratatui::{
    buffer::Cell,
    style::{Modifier, Style},
};
use tui_term::widget::{Cell as TerminalCell, Screen as TerminalScreen};

#[derive(Default)]
pub struct Replies {
    pub pending: Vec<u8>,
}
impl vt100::Callbacks for Replies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _: Option<u8>,
        params: &[&[u16]],
        command: char,
    ) {
        let n = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        let reply = match (command, n, i1) {
            ('n', 5, None) => "\x1b[0n".to_string(),
            ('n', 6, prefix) => {
                let (r, c) = screen.cursor_position();
                format!(
                    "\x1b[{}{};{}R",
                    if prefix == Some(b'?') { "?" } else { "" },
                    r + 1,
                    c + 1
                )
            }
            ('c', _, Some(b'>')) => "\x1b[>0;1;0c".into(),
            ('c', _, None) => "\x1b[?1;2c".into(),
            ('t', 18, None) => {
                let (r, c) = screen.size();
                format!("\x1b[8;{r};{c}t")
            }
            _ => String::new(),
        };
        self.pending.extend_from_slice(reply.as_bytes());
    }
}

pub type Emulator = vt100::Parser<Replies>;
pub fn emulator(rows: u16, cols: u16, history: usize) -> Emulator {
    vt100::Parser::new_with_callbacks(rows.max(1), cols.max(1), history, Replies::default())
}

pub struct ThemedCell {
    source: vt100::Cell,
    style: Style,
}
impl TerminalCell for ThemedCell {
    fn has_contents(&self) -> bool {
        true
    }
    fn apply(&self, target: &mut Cell) {
        target.set_symbol(if self.source.has_contents() {
            self.source.contents()
        } else {
            " "
        });
        target.set_style(self.style);
    }
}

pub struct ThemedScreen {
    cells: Vec<ThemedCell>,
    cols: u16,
    cursor: (u16, u16),
    hide: bool,
}
impl ThemedScreen {
    pub fn new(screen: &vt100::Screen, theme: &Theme, focused: bool) -> Self {
        let (rows, cols) = screen.size();
        let mut cells = Vec::with_capacity(rows as usize * cols as usize);
        for r in 0..rows {
            for c in 0..cols {
                let source = screen
                    .cell(r, c)
                    .expect("cell inside reported screen bounds")
                    .clone();
                let mut flags = Modifier::empty();
                for (enabled, flag) in [
                    (source.bold(), Modifier::BOLD),
                    (source.dim(), Modifier::DIM),
                    (source.italic(), Modifier::ITALIC),
                    (source.underline(), Modifier::UNDERLINED),
                    (source.inverse(), Modifier::REVERSED),
                ] {
                    if enabled {
                        flags |= flag;
                    }
                }
                let style = Style::reset()
                    .fg(theme.terminal_color(source.fgcolor(), true))
                    .bg(theme.terminal_color(source.bgcolor(), false))
                    .add_modifier(flags);
                cells.push(ThemedCell { source, style });
            }
        }
        let (r, c) = screen.cursor_position();
        Self {
            cells,
            cols,
            cursor: (
                r.saturating_add(screen.scrollback().try_into().unwrap_or(u16::MAX)),
                c,
            ),
            hide: screen.hide_cursor() || !focused || screen.scrollback() > 0,
        }
    }
}
impl TerminalScreen for ThemedScreen {
    type C = ThemedCell;
    fn cell(&self, row: u16, col: u16) -> Option<&Self::C> {
        if col >= self.cols {
            return None;
        }
        self.cells
            .get(row as usize * self.cols as usize + col as usize)
    }
    fn hide_cursor(&self) -> bool {
        self.hide
    }
    fn cursor_position(&self) -> (u16, u16) {
        self.cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunks_queries_alternate_screen_and_resize() {
        let mut p = emulator(10, 40, 20);
        let bytes = "中文\x1b[31mRED\x1b[0m".as_bytes();
        for byte in bytes {
            p.process(&[*byte]);
        }
        assert_eq!(p.screen().cell(0, 0).unwrap().contents(), "中");
        assert_eq!(
            p.screen().cell(0, 4).unwrap().fgcolor(),
            vt100::Color::Idx(1)
        );
        p.process(b"\x1b[3;7H\x1b[6n\x1b[5n\x1b[c");
        assert_eq!(p.callbacks().pending, b"\x1b[3;7R\x1b[0n\x1b[?1;2c");
        p.process(b"\x1b[?1049hALT");
        assert!(p.screen().alternate_screen());
        p.process(b"\x1b[?1049l");
        assert!(p.screen().contents().contains("中文"));
        p.screen_mut().set_size(5, 20);
        assert_eq!(p.screen().size(), (5, 20));
    }
    #[test]
    fn ansi_palette_is_applied_without_changing_rgb() {
        let mut p = emulator(2, 20, 0);
        p.process(b"\x1b[31mR\x1b[38;2;1;2;3mX");
        let t = crate::theme::builtin().remove(0);
        let screen = ThemedScreen::new(p.screen(), &t, true);
        assert_eq!(
            screen.cells[0].style.fg,
            Some(t.terminal_color(vt100::Color::Idx(1), true))
        );
        assert_eq!(
            screen.cells[1].style.fg,
            Some(ratatui::style::Color::Rgb(1, 2, 3))
        );
    }
}
