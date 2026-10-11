#![cfg(target_os = "linux")]
use rmux::adapters::protocol::{self, ClientMsg, ServerMsg};
use std::{
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

struct Server {
    child: Child,
    writer: ChildStdin,
    incoming: Receiver<std::io::Result<ServerMsg>>,
    directory: std::path::PathBuf,
    screen: vt100::Parser,
}
impl Server {
    fn start() -> Self {
        let directory = std::env::temp_dir().join(format!("rmux-ui-server-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("config.toml"), "[input]\nmouse = true\n").unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rmux"))
            .args(["serve", "--stdio", "--config"])
            .arg(directory.join("config.toml"))
            .args([
                "--shell",
                &std::env::var("RMUX_TEST_SHELL").unwrap_or_else(|_| "/bin/sh".into()),
            ])
            .env("RMUX_RUNTIME_DIR", &directory)
            .env_remove("RMUX_METRICS_ADDR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let writer = child.stdin.take().unwrap();
        let mut reader = child.stdout.take().unwrap();
        let (tx, incoming) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            loop {
                let msg = protocol::read_server(&mut reader);
                let done = msg.is_err();
                if tx.send(msg).is_err() || done {
                    break;
                }
            }
        });
        Self {
            child,
            writer,
            incoming,
            directory,
            screen: vt100::Parser::new(24, 80, 0),
        }
    }
    fn send(&mut self, msg: ClientMsg) {
        protocol::write_client(&mut self.writer, &msg).unwrap();
    }
    fn wait(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.screen.screen().contents().contains(text) {
            let msg = self
                .incoming
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
                .unwrap();
            if let ServerMsg::Frame { bytes, .. } = msg {
                self.screen.process(&bytes);
            }
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = protocol::write_client(&mut self.writer, &ClientMsg::Quit);
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            // Drain frames so the server cannot block behind the bounded reader queue.
            let _ = self.incoming.try_recv();
            std::thread::sleep(Duration::from_millis(5));
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
#[test]
fn server_actions_resize_reload_mouse_and_window_cleanup() {
    let mut s = Server::start();
    s.send(ClientMsg::Hello { rows: 24, cols: 80 });
    s.wait("1:shell*");
    s.send(ClientMsg::Write(
        b"stty -echo; printf 'ORIGINAL_'; printf 'SHELL\\n'\r".to_vec(),
    ));
    s.wait("ORIGINAL_SHELL");
    s.send(ClientMsg::Action(1));
    s.wait("pane 2");
    s.send(ClientMsg::Action(14));
    s.wait("pane 1");
    s.send(ClientMsg::Mouse {
        column: 60,
        row: 2,
        scroll: 0,
    });
    s.wait("pane 2");
    s.send(ClientMsg::Action(18));
    s.wait("2:shell*");
    s.send(ClientMsg::Action(20));
    s.wait("ORIGINAL_SHELL");
    std::fs::write(
        s.directory.join("config.toml"),
        "[input]\nmouse = true\n[pane]\nheader = true\npadding_x = 1\npadding_y = 1\n",
    )
    .unwrap();
    s.send(ClientMsg::Action(21));
    s.wait("Configuration reloaded");
    s.send(ClientMsg::Action(19));
    s.wait("2:shell*");
    s.send(ClientMsg::Write(
        b"stty -echo; stty size; printf 'SIZE_'; printf 'CHECK\\n'\r".to_vec(),
    ));
    s.wait("SIZE_CHECK");
    assert!(s.screen.screen().contents().contains("20 78"));
    s.send(ClientMsg::Write(b"exit\r".to_vec()));
    s.wait("1:shell*");
    s.send(ClientMsg::Quit);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = s.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "server failed to quit");
        let _ = s.incoming.try_recv();
        std::thread::sleep(Duration::from_millis(5));
    }
}
