use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Encode keyboard events as xterm input, respecting application cursor mode.
pub fn encode_key(key: KeyEvent, application_cursor: bool) -> Vec<u8> {
    let modifiers = key.modifiers;
    let special_modifier = 1
        + u8::from(modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(modifiers.contains(KeyModifiers::CONTROL));
    let cursor = |suffix: char| {
        if special_modifier > 1 {
            format!("\x1b[1;{special_modifier}{suffix}").into_bytes()
        } else {
            format!("\x1b{}{suffix}", if application_cursor { 'O' } else { '[' }).into_bytes()
        }
    };
    let tilde = |number: u8| {
        if special_modifier > 1 {
            format!("\x1b[{number};{special_modifier}~").into_bytes()
        } else {
            format!("\x1b[{number}~").into_bytes()
        }
    };
    let mut bytes = match key.code {
        KeyCode::Char(c) => {
            if modifiers.contains(KeyModifiers::CONTROL) {
                match c.to_ascii_lowercase() {
                    'a'..='z' => vec![c.to_ascii_lowercase() as u8 - b'a' + 1],
                    ' ' | '@' | '2' => vec![0],
                    '[' | '3' => vec![27],
                    '\\' | '4' => vec![28],
                    ']' | '5' => vec![29],
                    '^' | '6' => vec![30],
                    '_' | '/' | '7' => vec![31],
                    '?' | '8' => vec![127],
                    _ => return Vec::new(),
                }
            } else {
                c.to_string().into_bytes()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![127],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => return b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![27],
        KeyCode::Up => return cursor('A'),
        KeyCode::Down => return cursor('B'),
        KeyCode::Right => return cursor('C'),
        KeyCode::Left => return cursor('D'),
        KeyCode::Home => return cursor('H'),
        KeyCode::End => return cursor('F'),
        KeyCode::Insert => return tilde(2),
        KeyCode::Delete => return tilde(3),
        KeyCode::PageUp => return tilde(5),
        KeyCode::PageDown => return tilde(6),
        KeyCode::F(n @ 1..=4) => {
            let suffix = char::from(b'P' + n - 1);
            return if special_modifier > 1 {
                format!("\x1b[1;{special_modifier}{suffix}").into_bytes()
            } else {
                format!("\x1bO{suffix}").into_bytes()
            };
        }
        KeyCode::F(n @ 5..=12) => {
            return tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]);
        }
        _ => return Vec::new(),
    };
    if modifiers.contains(KeyModifiers::ALT) {
        bytes.insert(0, 27);
    }
    bytes
}

pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_and_application_keys() {
        let key = |code, mods| KeyEvent::new(code, mods);
        assert_eq!(
            encode_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL), false),
            [3]
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('界'), KeyModifiers::NONE), false),
            "界".as_bytes()
        );
        assert_eq!(
            encode_key(key(KeyCode::Up, KeyModifiers::NONE), true),
            b"\x1bOA"
        );
        assert_eq!(
            encode_key(key(KeyCode::Left, KeyModifiers::CONTROL), true),
            b"\x1b[1;5D"
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('x'), KeyModifiers::ALT), false),
            b"\x1bx"
        );
        assert_eq!(encode_paste("a\nb", false), b"a\rb");
        assert_eq!(encode_paste("a\nb", true), b"\x1b[200~a\nb\x1b[201~");
    }
}

/// Keyboard timing stays in the input adapter; the core receives only actions.
#[derive(Default)]
pub struct Bindings {
    pub prefix: bool,
    repeat_until: Option<std::time::Instant>,
}
pub enum Input {
    Prefix,
    Action(super::config::Action),
    Shell,
}
impl Bindings {
    pub fn key(
        &mut self,
        config: &super::config::Config,
        key: &KeyEvent,
        now: std::time::Instant,
    ) -> Input {
        if config.is_prefix(key) && !self.prefix {
            self.prefix = true;
            self.repeat_until = None;
            return Input::Prefix;
        }
        let prefixed = std::mem::take(&mut self.prefix);
        let repeating = self.repeat_until.is_some_and(|deadline| now < deadline);
        let action = if prefixed {
            config.binding(true, key)
        } else if repeating {
            config
                .binding(true, key)
                .filter(|a| super::config::Config::repeatable(a))
                .or_else(|| config.binding(false, key))
        } else {
            config.binding(false, key)
        };
        self.repeat_until = None;
        if let Some(action) = action {
            if super::config::Config::repeatable(action) && config.input.repeat_time_ms > 0 {
                self.repeat_until = now.checked_add(std::time::Duration::from_millis(
                    config.input.repeat_time_ms,
                ));
            }
            Input::Action(action.clone())
        } else if prefixed {
            Input::Action(super::config::Action::Cancel)
        } else {
            Input::Shell
        }
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    use crate::adapters::config::{Action, Config};
    #[test]
    fn repeated_focus_expires_and_ordinary_typing_exits_repeat_table() {
        let config = Config::default();
        let now = std::time::Instant::now();
        let mut input = Bindings::default();
        assert!(matches!(
            input.key(
                &config,
                &KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
                now
            ),
            Input::Prefix
        ));
        assert!(matches!(
            input.key(
                &config,
                &KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
                now
            ),
            Input::Action(Action::FocusLeft)
        ));
        assert!(matches!(
            input.key(
                &config,
                &KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
                now
            ),
            Input::Action(Action::FocusRight)
        ));
        assert!(matches!(
            input.key(
                &config,
                &KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
                now + std::time::Duration::from_millis(501)
            ),
            Input::Shell
        ));
        input.key(
            &config,
            &KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            now,
        );
        input.key(
            &config,
            &KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE),
            now,
        );
        assert!(matches!(
            input.key(
                &config,
                &KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
                now
            ),
            Input::Shell
        ));
        assert!(matches!(
            input.key(
                &config,
                &KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
                now
            ),
            Input::Shell
        ));
    }
}
