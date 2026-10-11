use std::{
    io::{self, Read, Write},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    thread,
};

use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

enum Output {
    Data(Vec<u8>),
    Closed,
    Error(io::Error),
}

pub struct Pane {
    master: Box<dyn MasterPty + Send>,
    input: SyncSender<Vec<u8>>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<Output>,
    parser: vt100::Parser,
    closed: bool,
}

impl Pane {
    pub fn spawn(shell: &str, rows: u16, cols: u16) -> Result<Self> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let pair = native_pty_system()
            .openpty(size(rows, cols))
            .context("open pseudo-terminal")?;
        // Acquire fallible resources before spawning, so setup errors cannot orphan a child.
        let mut reader = pair.master.try_clone_reader().context("clone PTY reader")?;
        let mut writer = pair.master.take_writer().context("open PTY writer")?;
        let mut command = CommandBuilder::new(shell);
        command.cwd(std::env::current_dir().context("get shell working directory")?);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("RMUX", "1");
        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("spawn shell {shell:?}"))?;
        drop(pair.slave);
        // At most 256 KiB of pending output per pane; a noisy process gets backpressure.
        let (sender, output) = mpsc::sync_channel(64);
        let (input, input_receiver) = mpsc::sync_channel::<Vec<u8>>(16);
        let pane = Self {
            master: pair.master,
            input,
            child,
            output,
            parser: vt100::Parser::new(rows, cols, 2000),
            closed: false,
        };
        let input_errors = sender.clone();
        thread::Builder::new()
            .name("rmux-pty-writer".into())
            .spawn(move || {
                for bytes in input_receiver {
                    if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
                        let _ = input_errors.send(Output::Error(error));
                        break;
                    }
                }
            })
            .context("start PTY writer")?;
        thread::Builder::new()
            .name("rmux-pty-reader".into())
            .spawn(move || {
                let mut buffer = [0; 4096];
                loop {
                    let message = match reader.read(&mut buffer) {
                        Ok(0) => Output::Closed,
                        Ok(count) => Output::Data(buffer[..count].to_vec()),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        // Linux PTYs may signal end-of-file with EIO after the slave closes.
                        Err(error)
                            if cfg!(target_os = "linux") && error.raw_os_error() == Some(5) =>
                        {
                            Output::Closed
                        }
                        Err(error) => Output::Error(error),
                    };
                    let done = !matches!(message, Output::Data(_));
                    if sender.send(message).is_err() || done {
                        break;
                    }
                }
            })
            .context("start PTY reader")?;
        Ok(pane)
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.parser.screen_mut().set_scrollback(0);
        anyhow::ensure!(bytes.len() <= 65_536, "input exceeds 64 KiB limit");
        match self.input.try_send(bytes.to_vec()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(anyhow::anyhow!("pane input queue is full")),
            Err(TrySendError::Disconnected(_)) => Err(anyhow::anyhow!("pane input writer closed")),
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if self.screen().size() != (rows, cols) {
            self.master.resize(size(rows, cols)).context("resize PTY")?;
            self.parser.screen_mut().set_size(rows, cols);
        }
        Ok(())
    }

    /// Bounded work keeps keyboard handling responsive under sustained output.
    pub fn drain(&mut self) -> Result<usize> {
        let mut received = 0;
        for _ in 0..64 {
            match self.output.try_recv() {
                Ok(Output::Data(bytes)) => {
                    received += bytes.len();
                    self.parser.process(&bytes);
                }
                Ok(Output::Closed) | Err(TryRecvError::Disconnected) => {
                    self.closed = true;
                    break;
                }
                Ok(Output::Error(error)) => return Err(error).context("read pane output"),
                Err(TryRecvError::Empty) => break,
            }
        }
        Ok(received)
    }

    pub fn exited(&mut self) -> Result<bool> {
        // Checking the child also handles shells whose descendants retain the slave FD.
        Ok(self.closed
            || self
                .child
                .try_wait()
                .context("check shell status")?
                .is_some())
    }

    pub fn scroll(&mut self, delta: isize) {
        let position = self.screen().scrollback().saturating_add_signed(delta);
        self.parser.screen_mut().set_scrollback(position);
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        if self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
    }
}

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}
