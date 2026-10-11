#![cfg(target_os = "linux")]

use nix::{
    fcntl::{FcntlArg, OFlag, fcntl},
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    thread,
    time::{Duration, Instant},
};

struct Client {
    _master: Box<dyn MasterPty + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    screen: vt100::Parser,
    output: Vec<u8>,
}

impl Client {
    fn start() -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let fd = pair.master.as_raw_fd().unwrap();
        let flags = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL).unwrap());
        fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).unwrap();
        let reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_rmux"));
        command.args([
            "--shell",
            &std::env::var("RMUX_TEST_SHELL").unwrap_or_else(|_| "/bin/sh".into()),
        ]);
        command.env("TERM", "xterm-256color");
        command.env_remove("RMUX_METRICS_ADDR");
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        Self {
            _master: pair.master,
            reader,
            writer,
            child,
            screen: vt100::Parser::new(24, 80, 0),
            output: Vec::new(),
        }
    }
    fn read(&mut self) {
        let mut buffer = [0; 4096];
        for _ in 0..64 {
            match self.reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    self.screen.process(&buffer[..count]);
                    self.output.extend_from_slice(&buffer[..count]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.raw_os_error() == Some(5) =>
                {
                    break;
                }
                Err(error) => panic!("outer PTY read: {error}"),
            }
        }
    }
    fn wait(&mut self, predicate: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.read();
            if predicate(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "terminal deadline exceeded: {:?}",
                self.screen.screen().contents()
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
    }
    fn contents(&self) -> String {
        self.screen.screen().contents()
    }
    fn finish(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.read();
            if self.child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "client did not shut down");
            thread::sleep(Duration::from_millis(5));
        }
        self.read();
        assert!(
            self.output
                .windows(b"\x1b[?1049l".len())
                .any(|window| window == b"\x1b[?1049l"),
            "alternate screen was not restored"
        );
        assert!(!self.screen.screen().alternate_screen());
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) && self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
    }
}

#[test]
fn attached_controls_shell_output_and_terminal_restoration() {
    let mut client = Client::start();
    client.wait(|client| client.contents().contains("pane 1"));
    client.send(b"printf 'END_'; printf 'TO_END'\r");
    client.wait(|client| client.contents().contains("END_TO_END"));
    client.send(
        concat!(
            r"printf '\033]52;c;blocked\007\033]2;blocked\007SAFE_'; printf 'RESULT'",
            "\r"
        )
        .as_bytes(),
    );
    client.wait(|client| client.contents().contains("SAFE_RESULT"));
    assert!(
        !client.output.windows(5).any(|window| window == b"\x1b]52;"),
        "child clipboard control escaped the emulator"
    );
    assert!(
        !client.output.windows(4).any(|window| window == b"\x1b]2;"),
        "child title control escaped the emulator"
    );
    client.send(b"\x02%");
    client.wait(|client| client.contents().contains("pane 2"));
    client.send(b"\x02z");
    client.wait(|client| {
        client.contents().contains("pane 2") && !client.contents().contains("pane 1")
    });
    client.send(b"\x02\"");
    client.wait(|client| client.contents().contains("pane 3"));
    client.send(b"\x02o");
    client.wait(|client| client.contents().contains("* pane 1"));
    client.send(b"\x02q");
    client.wait(|client| client.contents().contains("Quit and terminate"));
    client.send(b"n");
    client.wait(|client| {
        !client.contents().contains("Quit and terminate") && client.contents().contains("pane 3")
    });
    client.send(b"\x02q");
    client.wait(|client| client.contents().contains("Quit and terminate"));
    client.send(b"y");
    client.finish();
}

#[test]
fn sigterm_restores_terminal_and_exits_cleanly() {
    let mut client = Client::start();
    client.wait(|client| client.contents().contains("pane 1"));
    let pid = client.child.process_id().unwrap();
    kill(Pid::from_raw(pid.try_into().unwrap()), Signal::SIGTERM).unwrap();
    client.finish();
}
