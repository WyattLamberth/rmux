use std::{
    collections::BTreeMap,
    io::{self, Write},
};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    queue,
    style::{
        Attribute, Attributes, Color, Print, ResetColor, SetAttribute, SetAttributes,
        SetBackgroundColor, SetForegroundColor,
    },
    terminal::{BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate},
};
use rmux_core::{PaneId, Session};

use super::pty::Pane;

/// Render emulated cells, never forward untrusted child escape sequences to the host.
pub struct Chrome {
    pub status_fg: Color,
    pub status_bg: Color,
    pub pane_fg: Color,
    pub pane_bg: Color,
    pub active_fg: Color,
    pub active_bg: Color,
    pub top: bool,
}

pub fn draw(
    session: &Session,
    panes: &BTreeMap<PaneId, Pane>,
    dimensions: (u16, u16),
    status: &str,
    chrome: &Chrome,
) -> io::Result<Vec<u8>> {
    let mut frame = Vec::new();
    queue!(
        frame,
        BeginSynchronizedUpdate,
        Hide,
        SetAttribute(Attribute::Reset),
        ResetColor,
        Clear(ClearType::All)
    )?;
    let (width, height) = dimensions;
    let mut cursor = None;
    for (id, rect) in session.rectangles() {
        let Some(pane) = panes.get(&id) else {
            continue;
        };
        if rect.width == 0 || rect.height == 0 || rect.x >= width || rect.y >= height {
            continue;
        }
        let active = session.focused() == Some(id);
        if session.pane_layout().headers {
            queue!(
                frame,
                MoveTo(rect.x, rect.y),
                SetAttribute(Attribute::Reset),
                SetBackgroundColor(if active {
                    chrome.active_bg
                } else {
                    chrome.pane_bg
                }),
                SetForegroundColor(if active {
                    chrome.active_fg
                } else {
                    chrome.pane_fg
                })
            )?;
            let label = format!(
                " {} pane {}{} ",
                if active { "*" } else { " " },
                id,
                if pane.screen().scrollback() > 0 {
                    " [scroll]"
                } else {
                    ""
                }
            );
            print_line(&mut frame, &label, rect.width.min(width - rect.x))?;
        }
        let content = session.content_rect(rect);
        for row in 0..content.height.min(height.saturating_sub(content.y)) {
            queue!(frame, MoveTo(content.x, content.y + row))?;
            let mut previous = None;
            for col in 0..content.width.min(width.saturating_sub(content.x)) {
                let Some(cell) = pane.screen().cell(row, col) else {
                    break;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let style = (cell.fgcolor(), cell.bgcolor(), attributes(cell));
                if previous != Some(style) {
                    queue!(
                        frame,
                        SetAttribute(Attribute::Reset),
                        SetForegroundColor(color(style.0)),
                        SetBackgroundColor(color(style.1)),
                        SetAttributes(style.2)
                    )?;
                    previous = Some(style);
                }
                if cell.is_wide() && col + 1 >= content.width {
                    queue!(frame, Print(" "))?;
                } else {
                    queue!(
                        frame,
                        Print(if cell.has_contents() {
                            cell.contents()
                        } else {
                            " "
                        })
                    )?;
                }
            }
        }
        if active && !pane.screen().hide_cursor() && pane.screen().scrollback() == 0 {
            let (row, col) = pane.screen().cursor_position();
            if row < content.height && col < content.width {
                cursor = Some((content.x + col, content.y + row));
            }
        }
    }
    if width > 0 && height > 0 {
        queue!(
            frame,
            MoveTo(0, if chrome.top { 0 } else { height - 1 }),
            SetAttribute(Attribute::Reset),
            SetBackgroundColor(chrome.status_bg),
            SetForegroundColor(chrome.status_fg)
        )?;
        print_line(&mut frame, status, width)?;
    }
    queue!(frame, SetAttribute(Attribute::Reset), ResetColor)?;
    if let Some((x, y)) = cursor.filter(|(x, y)| {
        *x < width
            && *y < height
            && *y
                != if chrome.top {
                    0
                } else {
                    height.saturating_sub(1)
                }
    }) {
        queue!(frame, MoveTo(x, y), Show)?;
    }
    queue!(frame, EndSynchronizedUpdate)?;
    Ok(frame)
}

fn print_line(frame: &mut impl Write, text: &str, width: u16) -> io::Result<()> {
    // UI text is intentionally ASCII and one cell per character.
    let clipped: String = text
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_control() {
                c
            } else {
                '?'
            }
        })
        .take(usize::from(width))
        .collect();
    queue!(
        frame,
        Print(&clipped),
        Print(" ".repeat(usize::from(width) - clipped.len()))
    )
}

fn color(value: vt100::Color) -> Color {
    match value {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::AnsiValue(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb { r, g, b },
    }
}

fn attributes(cell: &vt100::Cell) -> Attributes {
    let mut result = Attributes::default();
    for (enabled, attribute) in [
        (cell.bold(), Attribute::Bold),
        (cell.dim(), Attribute::Dim),
        (cell.italic(), Attribute::Italic),
        (cell.underline(), Attribute::Underlined),
        (cell.inverse(), Attribute::Reverse),
    ] {
        if enabled {
            result.set(attribute);
        }
    }
    result
}
