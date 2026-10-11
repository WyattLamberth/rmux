//! Outer terminal ownership and cell-based rendering. Child escapes never reach stdout.
use crate::session::{MAX_COLS, MAX_ROWS, Session};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute, queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{
        self, Clear, ClearType, DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use std::io::{self, Write};

pub struct TerminalGuard;
impl TerminalGuard {
    pub fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            DisableLineWrap,
            Hide
        )?;
        Ok(guard)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            SetAttribute(Attribute::Reset),
            ResetColor,
            DisableBracketedPaste,
            EnableLineWrap,
            Show,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(n) => Color::AnsiValue(n),
        vt100::Color::Rgb(r, g, b) => Color::Rgb { r, g, b },
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Style {
    fg: vt100::Color,
    bg: vt100::Color,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    inverse: bool,
}
impl Style {
    fn of(cell: &vt100::Cell) -> Self {
        Self {
            fg: cell.fgcolor(),
            bg: cell.bgcolor(),
            bold: cell.bold(),
            dim: cell.dim(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        }
    }
    fn write(self, out: &mut impl Write) -> io::Result<()> {
        queue!(
            out,
            SetAttribute(Attribute::Reset),
            SetForegroundColor(color(self.fg)),
            SetBackgroundColor(color(self.bg))
        )?;
        for (enabled, attribute) in [
            (self.bold, Attribute::Bold),
            (self.dim, Attribute::Dim),
            (self.italic, Attribute::Italic),
            (self.underline, Attribute::Underlined),
            (self.inverse, Attribute::Reverse),
        ] {
            if enabled {
                queue!(out, SetAttribute(attribute))?;
            }
        }
        Ok(())
    }
}

pub fn render(
    out: &mut impl Write,
    session: &Session,
    cols: u16,
    rows: u16,
    notice: &str,
) -> io::Result<()> {
    queue!(
        out,
        Hide,
        SetAttribute(Attribute::Reset),
        ResetColor,
        Clear(ClearType::All)
    )?;
    let mut style = None;
    for (id, rect) in session.rectangles() {
        if !rect.visible() {
            continue;
        }
        if let Some(pane) = session.panes().get(&id) {
            for row in 0..rect.rows {
                queue!(out, MoveTo(rect.x, rect.y + row))?;
                for col in 0..rect.cols {
                    if let Some(cell) = pane.terminal.screen().cell(row, col) {
                        if cell.is_wide_continuation() {
                            continue;
                        }
                        let current = Style::of(cell);
                        if style != Some(current) {
                            current.write(out)?;
                            style = Some(current);
                        }
                        // A truncated wide glyph must not spill into a neighboring pane.
                        if cell.has_contents() && (!cell.is_wide() || col + 1 < rect.cols) {
                            queue!(out, Print(cell.contents()))?;
                        } else {
                            queue!(out, Print(" "))?;
                        }
                    }
                }
            }
        }
    }
    if rows > 0 && cols > 0 {
        let focus = session
            .focus()
            .map(|id| id.0.to_string())
            .unwrap_or_else(|| "-".into());
        let state = session
            .focus()
            .and_then(|id| session.panes().get(&id))
            .map(|p| format!("{:?}", p.lifecycle))
            .unwrap_or_default();
        let text = format!(
            "rmux pane {focus} {state}{} | {notice}",
            if session.zoomed() { " zoom" } else { "" }
        );
        queue!(
            out,
            SetAttribute(Attribute::Reset),
            ResetColor,
            SetAttribute(Attribute::Reverse),
            MoveTo(0, rows.min(MAX_ROWS + 1) - 1),
            Print(
                text.chars()
                    .take(usize::from(cols.min(MAX_COLS)))
                    .collect::<String>()
            ),
            SetAttribute(Attribute::Reset)
        )?;
    }
    for (id, rect) in session.rectangles() {
        if Some(id) == session.focus() && rect.visible() {
            if let Some(pane) = session.panes().get(&id) {
                let screen = pane.terminal.screen();
                let (row, col) = screen.cursor_position();
                if !screen.hide_cursor()
                    && screen.scrollback() == 0
                    && row < rect.rows
                    && col < rect.cols
                {
                    queue!(out, MoveTo(rect.x + col, rect.y + row), Show)?;
                }
            }
        }
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Axis;
    use crate::session::{Event, PaneId};
    #[test]
    fn child_cursor_and_erases_cannot_escape_pane() -> io::Result<()> {
        let (mut session, _) = Session::new(21, 5);
        session.apply(Event::Spawned(PaneId(0)));
        session.apply(Event::Split(Axis::Columns));
        session.apply(Event::Spawned(PaneId(1)));
        session.apply(Event::Output(
            PaneId(0),
            b"left\x1b[999;999H\x1b[2Jok".to_vec(),
        ));
        session.apply(Event::Output(PaneId(1), "right界".as_bytes().to_vec()));
        let mut bytes = Vec::new();
        render(&mut bytes, &session, 21, 5, "test")?;
        let mut outer = crate::terminal::Terminal::new(5, 21, 0);
        outer.process(&bytes);
        assert!(outer.screen().contents().contains("right界"));
        assert!(outer.screen().contents().contains("rmux pane"));
        for cols in 0..3 {
            for rows in 0..3 {
                session.apply(Event::Resize { cols, rows });
                render(&mut Vec::new(), &session, cols, rows, "test")?;
            }
        }
        Ok(())
    }
}
