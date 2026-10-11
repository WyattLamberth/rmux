//! Discard unsupported OSC/DCS/APC/PM strings without retaining their payload.
//! vt100's upstream VTE parser otherwise grows an OSC Vec until its terminator.
#[derive(Clone, Copy, Default)]
enum Filter {
    #[default]
    Text,
    Escape,
    String,
    StringEscape,
}

pub struct Terminal {
    parser: vt100::Parser,
    filter: Filter,
}
impl Terminal {
    pub fn new(rows: u16, cols: u16, history: usize) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, history),
            filter: Filter::Text,
        }
    }
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }
    pub fn screen_mut(&mut self) -> &mut vt100::Screen {
        self.parser.screen_mut()
    }
    pub fn process(&mut self, bytes: &[u8]) {
        let mut visible = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            self.filter = match (self.filter, byte) {
                (Filter::Text, 27) => Filter::Escape,
                (Filter::Text, _) => {
                    visible.push(byte);
                    Filter::Text
                }
                (Filter::Escape, b']' | b'P' | b'_' | b'^' | b'X') => Filter::String,
                (Filter::Escape, 27) => Filter::Escape,
                (Filter::Escape, _) => {
                    visible.extend([27, byte]);
                    Filter::Text
                }
                (Filter::String | Filter::StringEscape, 7 | 24 | 26) => Filter::Text,
                (Filter::String, 27) => Filter::StringEscape,
                (Filter::StringEscape, b'\\') => Filter::Text,
                (Filter::StringEscape, 27) => Filter::StringEscape,
                (Filter::String | Filter::StringEscape, _) => Filter::String,
            };
        }
        self.parser.process(&visible);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unterminated_strings_do_not_retain_payload_and_recover_across_chunks() {
        let mut terminal = Terminal::new(3, 30, 4);
        terminal.process(b"before\x1b]");
        for _ in 0..1024 {
            terminal.process(&[b'x'; 4096]);
        }
        terminal.process(b"\x1b");
        terminal.process(b"\\after\x1bPignored\x1b\\end");
        assert_eq!(terminal.screen().contents(), "beforeafterend");
        terminal.process("界e\u{301}".as_bytes());
        assert!(terminal.screen().contents().contains("界e\u{301}"));
    }
}
