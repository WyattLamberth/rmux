use rmux::adapters::{
    config::{self, Config},
    pty::Pane,
    render::{self, Chrome},
};
use rmux_core::{Event, Rect, Session};
use std::collections::BTreeMap;
fn chrome(config: &Config) -> Chrome {
    let color = |name: &str| config::named_color(name).unwrap();
    Chrome {
        message_fg: color(&config.style.message_fg),
        message_bg: color(&config.style.message_bg),
        status_fg: color(&config.style.status_fg),
        status_bg: color(&config.style.status_bg),
        pane_fg: color(&config.style.pane_fg),
        pane_bg: color(&config.style.pane_bg),
        active_fg: color(&config.style.active_fg),
        active_bg: color(&config.style.active_bg),
        border_fg: color(&config.style.pane_fg),
        active_border_fg: color(&config.style.active_bg),
        top: config.status.top,
        headers: config.pane.headers,
        left: config.status_left(),
        right: config.status_right(),
        left_length: config.status.left_length,
        right_length: config.status.right_length,
    }
}
#[test]
fn rose_pine_cells_optional_headers_and_tiny_top_bottom_status() {
    crossterm::style::force_color_output(true);
    let shell = std::env::var("RMUX_TEST_SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let panes = BTreeMap::from([(1, Pane::spawn(&shell, 24, 80).unwrap())]);
    let mut config = Config::default();
    for headers in [false, true] {
        for top in [false, true] {
            for (width, height) in [(0u16, 0u16), (1, 1), (2, 2), (4, 3), (80, 24)] {
                config.pane.headers = headers;
                config.status.top = top;
                let mut session = Session::new(Rect {
                    x: 0,
                    y: u16::from(top),
                    width,
                    height: height.saturating_sub(1),
                });
                session.apply(Event::Chrome(config.pane.chrome()));
                let frame =
                    render::draw(&session, &panes, (width, height), "", &chrome(&config)).unwrap();
                let mut screen = vt100::Parser::new(height.max(2), width.max(2), 0);
                screen.process(&frame);
                if width == 80 && height == 24 {
                    let status_row = if top { 0 } else { 23 };
                    assert!(
                        screen
                            .screen()
                            .rows(0, 80)
                            .nth(status_row)
                            .unwrap()
                            .contains("1:shell*")
                    );
                    assert_eq!(
                        screen
                            .screen()
                            .cell(status_row as u16, 1)
                            .unwrap()
                            .bgcolor(),
                        vt100::Color::Rgb(196, 167, 231),
                        "headers={headers} top={top} rgb sequences={}",
                        frame.windows(6).filter(|b| *b == b"48;2;1").count()
                    );
                    assert_eq!(
                        screen
                            .screen()
                            .rows(0, 80)
                            .nth(usize::from(top))
                            .unwrap()
                            .contains("* pane 1"),
                        headers
                    );
                }
            }
        }
    }
}
