#![cfg(target_os = "linux")]

use rmux::adapters::pty::Pane;
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn wait_for(pane: &mut Pane, predicate: impl Fn(&Pane) -> bool) {
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

#[test]
fn real_shell_resize_unicode_color_and_exit() {
    let mut pane = Pane::spawn("/bin/sh", 24, 80).unwrap();
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
    let pane = Pane::spawn("/bin/sh", 10, 30).unwrap();
    let pid = pane.process_id().expect("Linux child PID");
    drop(pane);
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "shell was not reaped"
    );
}

#[test]
fn input_backpressure_never_blocks_ui_thread() {
    let mut pane = Pane::spawn("/bin/sh", 10, 30).unwrap();
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
