//! Keyboard encoding and prefix commands; no OS operations.
use crate::{
    layout::Axis,
    session::{Event, MAX_INPUT},
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub enum Action {
    Session(Event),
    Quit,
    Notice(&'static str),
}

#[derive(Default)]
pub struct Input {
    prefix: bool,
    scrolling: bool,
}

impl Input {
    pub fn key(&mut self, key: KeyEvent, screen: Option<&vt100::Screen>) -> Option<Action> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if self.prefix {
            self.prefix = false;
            let event = match key.code {
                KeyCode::Char('%') => Event::Split(Axis::Columns),
                KeyCode::Char('"') => Event::Split(Axis::Rows),
                KeyCode::Char('o') => Event::FocusNext,
                KeyCode::Char('z') => Event::ToggleZoom,
                KeyCode::Char('x') => Event::Close,
                KeyCode::Char('q') => return Some(Action::Quit),
                KeyCode::Char('[') => {
                    self.scrolling = true;
                    Event::Scroll(10)
                }
                KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Event::Input(vec![2])
                }
                _ => {
                    return Some(Action::Notice(
                        "prefix: % split columns | \" split rows | o focus | z zoom | [ history | x close | q quit",
                    ));
                }
            };
            return Some(Action::Session(event));
        }
        if key.code == KeyCode::Char('b') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.prefix = true;
            return Some(Action::Notice(
                "prefix: % split columns | \" split rows | o focus | z zoom | [ history | x close | q quit",
            ));
        }
        if self.scrolling {
            let delta = match key.code {
                KeyCode::PageUp => 10,
                KeyCode::PageDown => -10,
                KeyCode::Up => 1,
                KeyCode::Down => -1,
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.scrolling = false;
                    i32::MIN
                }
                _ => return None,
            };
            return Some(Action::Session(Event::Scroll(delta)));
        }
        encode_key(key, screen.is_some_and(vt100::Screen::application_cursor))
            .map(|bytes| Action::Session(Event::Input(bytes)))
    }

    pub fn paste(&mut self, text: &str, bracketed: bool) -> Action {
        // Reject oversize pastes as a unit rather than silently inserting a prefix.
        if text.len() + if bracketed { 12 } else { 0 } > MAX_INPUT {
            return Action::Notice("paste rejected: exceeds 16 KiB input limit");
        }
        self.prefix = false;
        self.scrolling = false;
        let mut bytes = Vec::with_capacity(text.len() + 12);
        if bracketed {
            bytes.extend(b"\x1b[200~");
        }
        bytes.extend(text.replace('\n', "\r").as_bytes());
        if bracketed {
            bytes.extend(b"\x1b[201~");
        }
        Action::Session(Event::Input(bytes))
    }
}

pub fn encode_key(key: KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    use KeyCode::*;
    let mods = key.modifiers;
    let modifier = 1
        + u8::from(mods.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(mods.contains(KeyModifiers::ALT))
        + 4 * u8::from(mods.contains(KeyModifiers::CONTROL));
    let mut bytes = match key.code {
        Char(c) => {
            if mods.contains(KeyModifiers::CONTROL) {
                match c {
                    '@' | ' ' | '2' => vec![0],
                    'a'..='z' | 'A'..='Z' => vec![(c as u8) & 0x1f],
                    '['..='_' => vec![(c as u8) & 0x1f],
                    '?' | '8' => vec![127],
                    _ => return None,
                }
            } else {
                c.to_string().into_bytes()
            }
        }
        Enter => vec![b'\r'],
        Tab => vec![b'\t'],
        Backspace => vec![127],
        Esc => vec![27],
        BackTab => b"\x1b[Z".to_vec(),
        Up | Down | Right | Left | Home | End => {
            let final_char = match key.code {
                Up => 'A',
                Down => 'B',
                Right => 'C',
                Left => 'D',
                Home => 'H',
                _ => 'F',
            };
            if modifier > 1 {
                format!("\x1b[1;{modifier}{final_char}").into_bytes()
            } else {
                format!(
                    "\x1b{}{final_char}",
                    if application_cursor { 'O' } else { '[' }
                )
                .into_bytes()
            }
        }
        Insert | Delete | PageUp | PageDown | F(5..=12) => {
            let number = match key.code {
                Insert => 2,
                Delete => 3,
                PageUp => 5,
                PageDown => 6,
                F(5) => 15,
                F(6) => 17,
                F(7) => 18,
                F(8) => 19,
                F(9) => 20,
                F(10) => 21,
                F(11) => 23,
                _ => 24,
            };
            if modifier > 1 {
                format!("\x1b[{number};{modifier}~").into_bytes()
            } else {
                format!("\x1b[{number}~").into_bytes()
            }
        }
        F(1..=4) => {
            let final_char = match key.code {
                F(1) => 'P',
                F(2) => 'Q',
                F(3) => 'R',
                _ => 'S',
            };
            if modifier > 1 {
                format!("\x1b[1;{modifier}{final_char}").into_bytes()
            } else {
                format!("\x1bO{final_char}").into_bytes()
            }
        }
        _ => return None,
    };
    if mods.contains(KeyModifiers::ALT)
        && matches!(key.code, Char(_) | Enter | Tab | Backspace | Esc)
    {
        bytes.insert(0, 27);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_unicode_application_cursor_and_modifiers() {
        assert_eq!(
            encode_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                false
            ),
            Some(vec![3])
        );
        assert_eq!(
            encode_key(
                KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE),
                false
            ),
            Some("界".as_bytes().to_vec())
        );
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode_key(
                KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL | KeyModifiers::ALT),
                true
            ),
            Some(b"\x1b[1;7D".to_vec())
        );
    }
    #[test]
    fn prefix_and_paste() {
        let mut input = Input::default();
        input.key(
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            None,
        );
        assert!(matches!(
            input.key(KeyEvent::new(KeyCode::Char('%'), KeyModifiers::SHIFT), None),
            Some(Action::Session(Event::Split(Axis::Columns)))
        ));
        assert!(
            matches!(input.paste("x\ny", true), Action::Session(Event::Input(bytes)) if bytes == b"\x1b[200~x\ry\x1b[201~")
        );
        assert!(matches!(
            input.paste(&"x".repeat(MAX_INPUT), true),
            Action::Notice(_)
        ));
    }
}
