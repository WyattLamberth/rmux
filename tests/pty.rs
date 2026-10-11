#![cfg(target_os = "linux")]

use rmux::adapters::pty::Pane;

fn shell() -> String {
    std::env::var("RMUX_TEST_SHELL").unwrap_or_else(|_| "/bin/sh".into())
}
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn wait_for(pane: &mut Pane, mut predicate: impl FnMut(&Pane) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        pane.drain().expect("PTY output");
        if predicate(pane) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "PTY deadline exceeded: {:?}",
            pane.screen().contents()
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn background_job_pid(contents: &str) -> Option<u32> {
    contents
        .split("JOB_")
        .skip(1)
        .filter_map(|rest| rest.split_once("_READY").map(|(pid, _)| pid))
        .find_map(|pid| pid.parse().ok())
}

#[test]
fn real_shell_resize_unicode_color_and_exit() {
    let mut pane = Pane::spawn(&shell(), 24, 80).unwrap();
    pane.resize(12, 50).unwrap();
    pane.write(concat!(r"stty -echo; stty size; printf '\033[31mPTY_'; printf 'READY\033[0m'; printf '\n\347\225\214\n'", "\r").as_bytes()).unwrap();
    wait_for(&mut pane, |pane| {
        pane.screen().contents().contains("PTY_READY") && pane.screen().contents().contains("12 50")
    });
    assert!(pane.screen().contents().contains('界'));
    assert_eq!(pane.screen().size(), (12, 50));
    let screen = pane.screen();
    assert!((0..12).any(|row| (0..50).any(|col| {
        screen
            .cell(row, col)
            .is_some_and(|cell| cell.fgcolor() == vt100::Color::Idx(1) && cell.contents() == "P")
    })));
    pane.write(b"exit\r").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !pane.exited().unwrap() {
        pane.drain().unwrap();
        assert!(Instant::now() < deadline, "shell did not exit");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn failed_spawn_and_drop_cleanup() {
    assert!(Pane::spawn("/this/executable/does/not/exist", 10, 30).is_err());
    let pane = Pane::spawn(&shell(), 10, 30).unwrap();
    let pid = pane.process_id().expect("Linux child PID");
    drop(pane);
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "shell was not reaped"
    );
}

#[test]
fn input_backpressure_never_blocks_ui_thread() {
    let mut pane = Pane::spawn(&shell(), 10, 30).unwrap();
    pane.write(b"exec sleep 60\r").unwrap();
    thread::sleep(Duration::from_millis(100));
    let payload = vec![b'x'; 65_536];
    let started = Instant::now();
    let mut full = false;
    for _ in 0..100 {
        if pane.write(&payload).is_err() {
            full = true;
            break;
        }
    }
    assert!(full, "input queue was not bounded");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "UI thread blocked on PTY write"
    );
    assert!(pane.write(&vec![b'x'; 65_537]).is_err());
}

#[test]
fn tiny_pane_with_wide_unicode_does_not_panic() {
    let mut pane = Pane::spawn(&shell(), 1, 1).unwrap();
    assert_eq!(pane.screen().size(), (2, 2));
    pane.resize(1, 0).unwrap();
    pane.write(concat!(r"stty -echo; printf '\347\225\214'; sleep 1", "\r").as_bytes())
        .unwrap();
    wait_for(&mut pane, |pane| pane.screen().contents().contains('界'));
}

#[test]
fn close_joins_workers_and_terminates_background_jobs() {
    let mut pane = Pane::spawn(&shell(), 10, 60).unwrap();
    pane.write(b"stty -echo; sleep 60 & printf 'JOB_%s_READY' $!\r")
        .unwrap();
    let mut job = None;
    wait_for(&mut pane, |pane| {
        job = background_job_pid(&pane.screen().contents());
        job.is_some()
    });
    let job = job.expect("shell should print the background job PID");
    let started = Instant::now();
    drop(pane);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "worker join blocked"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{job}/stat"));
        if stat.is_err()
            || stat
                .unwrap()
                .rsplit_once(')')
                .unwrap()
                .1
                .split_whitespace()
                .next()
                == Some("Z")
        {
            break;
        }
        assert!(Instant::now() < deadline, "background job still running");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn device_status_query_is_answered_to_the_shell() {
    let mut pane = Pane::spawn(&shell(), 24, 80).unwrap();
    pane.write(
        b"stty -echo -icanon min 1 time 0; (printf '\\033[5n'); dd bs=1 count=4 2>/dev/null | od -An -tx1; printf '\\nDONE\\n'\r",
    )
    .unwrap();
    wait_for(&mut pane, |pane| {
        let text = pane.screen().contents();
        text.contains("1b") && text.contains("5b") && text.contains("30") && text.contains("6e")
    });
}

#[test]
fn tiny_configured_viewports_render_without_showing_an_outside_cursor() {
    use crossterm::style::Color;
    use rmux::adapters::render::{Chrome, draw};
    use rmux_core::{Limits, PaneLayout, Rect, Session};
    use std::collections::BTreeMap;
    let pane = Pane::spawn(&shell(), 2, 2).unwrap();
    let panes = BTreeMap::from([(1, pane)]);
    for headers in [false, true] {
        for padding in [0, 1, 16] {
            for top in [false, true] {
                let chrome = Chrome {
                    status_fg: Color::White,
                    status_bg: Color::Blue,
                    pane_fg: Color::White,
                    pane_bg: Color::Black,
                    active_fg: Color::White,
                    active_bg: Color::Black,
                    top,
                };
                for width in 0..5 {
                    for height in 0u16..5 {
                        let session = Session::with_options(
                            Rect {
                                x: 0,
                                y: u16::from(top && height > 0),
                                width,
                                height: height.saturating_sub(1),
                            },
                            Limits::default(),
                            PaneLayout::new(headers, padding).unwrap(),
                        );
                        let frame = draw(&session, &panes, (width, height), "S", &chrome).unwrap();
                        let mut screen = vt100::Parser::new(height.max(2), width.max(2), 0);
                        screen.process(&frame);
                        if width > 0 && height > 0 {
                            assert_eq!(
                                screen
                                    .screen()
                                    .cell(if top { 0 } else { height - 1 }, 0)
                                    .unwrap()
                                    .contents(),
                                "S"
                            );
                        }
                        let content = session.content_rect(session.rectangles()[0].1);
                        if content.width == 0 || content.height == 0 {
                            assert!(screen.screen().hide_cursor());
                        }
                    }
                }
            }
        }
    }
}
