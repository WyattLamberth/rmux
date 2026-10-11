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
    pub header: bool,
    pub border_fg: Color,
    pub active_border_fg: Color,
    pub left: String,
    pub right: String,
    pub left_length: u16,
    pub right_length: u16,
    pub message_fg: Color,
    pub message_bg: Color,
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
        if chrome.header {
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
        let content = session.content(rect);
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
    // Separators occupy layout gaps, never PTY cells. Color a gap when it
    // touches the active pane so focus remains visible with headers disabled.
    let rects = session.rectangles();
    for (_, rect) in &rects {
        for (x, y, vertical) in [
            (rect.x.saturating_add(rect.width), rect.y, true),
            (rect.x, rect.y.saturating_add(rect.height), false),
        ] {
            let count = if vertical { rect.height } else { rect.width };
            for offset in 0..count {
                let (x, y) = if vertical {
                    (x, y.saturating_add(offset))
                } else {
                    (x.saturating_add(offset), y)
                };
                if x >= width
                    || y >= height
                    || y == if chrome.top {
                        0
                    } else {
                        height.saturating_sub(1)
                    }
                    || rects.iter().any(|(_, r)| contains(*r, x, y))
                {
                    continue;
                }
                let active = rects.iter().any(|(id, r)| {
                    Some(*id) == session.focused()
                        && (contains(*r, x.saturating_sub(1), y)
                            || contains(*r, x.saturating_add(1), y)
                            || contains(*r, x, y.saturating_sub(1))
                            || contains(*r, x, y.saturating_add(1)))
                });
                queue!(
                    frame,
                    MoveTo(x, y),
                    SetAttribute(Attribute::Reset),
                    SetBackgroundColor(chrome.status_bg),
                    SetForegroundColor(if active {
                        chrome.active_border_fg
                    } else {
                        chrome.border_fg
                    }),
                    Print(if vertical { "│" } else { "─" })
                )?;
            }
        }
    }
    if width > 0 && height > 0 {
        let y = if chrome.top { 0 } else { height - 1 };
        queue!(
            frame,
            MoveTo(0, y),
            SetAttribute(Attribute::Reset),
            SetBackgroundColor(chrome.status_bg),
            SetForegroundColor(chrome.status_fg)
        )?;
        print_line(&mut frame, "", width)?;
        if !status.is_empty() {
            queue!(
                frame,
                MoveTo(0, y),
                SetBackgroundColor(chrome.message_bg),
                SetForegroundColor(chrome.message_fg)
            )?;
            print_line(&mut frame, status, width)?;
        } else {
            let left = ascii(&chrome.left);
            let left_width = left
                .len()
                .min(usize::from(width))
                .min(usize::from(chrome.left_length)) as u16;
            queue!(
                frame,
                MoveTo(0, y),
                SetBackgroundColor(chrome.active_bg),
                SetForegroundColor(chrome.active_fg),
                SetAttribute(Attribute::Bold)
            )?;
            print_line(&mut frame, &left, left_width)?;
            let context = format!(
                " pane {}{}{}",
                session.focused().unwrap_or(0),
                if session.is_zoomed() { " [zoom]" } else { "" },
                if session
                    .focused()
                    .and_then(|id| panes.get(&id))
                    .is_some_and(|p| p.screen().scrollback() > 0)
                {
                    " [scroll]"
                } else {
                    ""
                }
            );
            let clock = chrono::Local::now().format(&chrome.right).to_string();
            let right = ascii(&format!("{context}{clock}"));
            let right_width = right
                .len()
                .min(usize::from(chrome.right_length))
                .min(usize::from(width.saturating_sub(left_width)))
                as u16;
            let right_x = width - right_width;
            queue!(
                frame,
                MoveTo(right_x, y),
                SetAttribute(Attribute::Reset),
                SetBackgroundColor(chrome.status_bg),
                SetForegroundColor(chrome.status_fg)
            )?;
            // Keep the clock/date when a small terminal forces clipping.
            print_line(
                &mut frame,
                &right[right.len() - usize::from(right_width)..],
                right_width,
            )?;
            let mut x = left_width;
            for id in session.window_ids() {
                if x >= right_x {
                    break;
                }
                let label = format!(
                    " {id}:shell{} ",
                    if id == session.active_window() {
                        "*"
                    } else {
                        ""
                    }
                );
                let w = (label.len() as u16).min(right_x - x);
                let active = id == session.active_window();
                queue!(
                    frame,
                    MoveTo(x, y),
                    SetAttribute(Attribute::Reset),
                    SetBackgroundColor(if active {
                        chrome.active_bg
                    } else {
                        chrome.status_bg
                    }),
                    SetForegroundColor(if active {
                        chrome.active_fg
                    } else {
                        chrome.status_fg
                    })
                )?;
                print_line(&mut frame, &label, w)?;
                x += w;
            }
        }
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

fn contains(rect: rmux_core::Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}
fn ascii(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii() && !c.is_control() {
                c
            } else {
                '?'
            }
        })
        .collect()
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
