use std::{
    io::{self, Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
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
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}

impl Pane {
    pub fn spawn(shell: &str, rows: u16, cols: u16) -> Result<Self> {
        let rows = rows.max(2);
        // vt100 0.16 requires at least two rows/columns for wrapping wide text.
        let cols = cols.max(2);
        let pair = native_pty_system()
            .openpty(size(rows, cols))
            .context("open pseudo-terminal")?;
        make_nonblocking(pair.master.as_ref())?;
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
        let stop = Arc::new(AtomicBool::new(false));
        let mut pane = Self {
            master: pair.master,
            input,
            child,
            output,
            parser: vt100::Parser::new(rows, cols, 2000),
            closed: false,
            stop: Arc::clone(&stop),
            workers: Vec::new(),
        };
        let input_errors = sender.clone();
        let writer_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("rmux-pty-writer".into())
            .spawn(move || {
                while !writer_stop.load(Ordering::Relaxed) {
                    let bytes = match input_receiver.recv_timeout(Duration::from_millis(5)) {
                        Ok(bytes) => bytes,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let mut remaining = bytes.as_slice();
                    while !remaining.is_empty() && !writer_stop.load(Ordering::Relaxed) {
                        match writer.write(remaining) {
                            Ok(0) => {
                                send_output(
                                    &input_errors,
                                    Output::Error(io::ErrorKind::WriteZero.into()),
                                    &writer_stop,
                                );
                                return;
                            }
                            Ok(count) => remaining = &remaining[count..],
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(2))
                            }
                            Err(error) => {
                                send_output(&input_errors, Output::Error(error), &writer_stop);
                                return;
                            }
                        }
                    }
                }
            })
            .context("start PTY writer")?;
        pane.workers.push(worker);
        let worker = thread::Builder::new()
            .name("rmux-pty-reader".into())
            .spawn(move || {
                let mut buffer = [0; 4096];
                while !stop.load(Ordering::Relaxed) {
                    let message = match reader.read(&mut buffer) {
                        Ok(0) => Output::Closed,
                        Ok(count) => Output::Data(buffer[..count].to_vec()),
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        // Linux PTYs may signal end-of-file with EIO after the slave closes.
                        Err(error)
                            if cfg!(target_os = "linux") && error.raw_os_error() == Some(5) =>
                        {
                            Output::Closed
                        }
                        Err(error) => Output::Error(error),
                    };
                    let done = !matches!(message, Output::Data(_));
                    if !send_output(&sender, message, &stop) || done {
                        break;
                    }
                }
            })
            .context("start PTY reader")?;
        pane.workers.push(worker);
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
        let rows = rows.max(2);
        let cols = cols.max(2);
        if self.screen().size() != (rows, cols) {
            self.master.resize(size(rows, cols)).context("resize PTY")?;
            self.parser.screen_mut().set_size(rows, cols);
        }
        Ok(())
    }

    /// Bounded work keeps keyboard handling responsive under sustained output.
    pub fn drain(&mut self) -> Result<usize> {
        self.drain_budget(64)
    }

    pub fn drain_budget(&mut self, chunks: usize) -> Result<usize> {
        let mut received = 0;
        for _ in 0..chunks.min(64) {
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
        // Observe without reaping: preserve PID/session identity until cleanup signals
        // all terminal session process groups, including jobs retaining the slave FD.
        if self.closed {
            return Ok(true);
        }
        child_exited(self.child.as_ref())
    }

    pub fn scroll(&mut self, delta: isize) {
        let position = self.screen().scrollback().saturating_add_signed(delta);
        self.parser.screen_mut().set_scrollback(position);
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        terminate_session(self.child.as_ref());
        if !matches!(self.child.try_wait(), Ok(Some(_))) && self.child.kill().is_ok() {
            let _ = self.child.wait();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn send_output(sender: &SyncSender<Output>, mut output: Output, stop: &AtomicBool) -> bool {
    while !stop.load(Ordering::Relaxed) {
        match sender.try_send(output) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(pending)) => {
                output = pending;
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
    false
}

#[cfg(target_os = "linux")]
fn make_nonblocking(master: &dyn MasterPty) -> Result<()> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    let fd = master.as_raw_fd().context("Linux PTY file descriptor")?;
    let flags = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL)?);
    fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).context("set cancellable PTY IO")?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn child_exited(child: &dyn Child) -> Result<bool> {
    use nix::{
        sys::wait::{Id, WaitPidFlag, WaitStatus, waitid},
        unistd::Pid,
    };
    let pid = child.process_id().context("shell PID")?;
    let status = waitid(
        Id::Pid(Pid::from_raw(pid.try_into()?)),
        WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT,
    )
    .context("check shell status")?;
    Ok(status != WaitStatus::StillAlive)
}

#[cfg(target_os = "linux")]
fn terminate_session(child: &dyn Child) {
    use nix::{
        sys::signal::{Signal, killpg},
        unistd::{Pid, getpgrp},
    };
    let Some(pid) = child.process_id().and_then(|pid| i32::try_from(pid).ok()) else {
        return;
    };
    // The child remains unreaped, so its identity cannot be recycled during this scan.
    let Ok(root) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return;
    };
    let Some(root_fields) = root
        .rsplit_once(')')
        .map(|(_, fields)| fields.split_whitespace().collect::<Vec<_>>())
    else {
        return;
    };
    if root_fields
        .get(3)
        .and_then(|field| field.parse::<i32>().ok())
        != Some(pid)
    {
        return;
    }
    let mut groups = std::collections::BTreeSet::from([pid]);
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
                continue;
            }
            let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            let Some((_, fields)) = stat.rsplit_once(')') else {
                continue;
            };
            let fields: Vec<_> = fields.split_whitespace().collect();
            if fields.get(3).and_then(|field| field.parse::<i32>().ok()) == Some(pid)
                && let Some(group) = fields.get(2).and_then(|field| field.parse::<i32>().ok())
            {
                groups.insert(group);
            }
        }
    }
    for group in groups {
        if group > 0 && group != getpgrp().as_raw() {
            let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn make_nonblocking(_: &dyn MasterPty) -> Result<()> {
    anyhow::bail!("this release supports Linux; macOS is planned")
}
#[cfg(not(target_os = "linux"))]
fn child_exited(_: &dyn Child) -> Result<bool> {
    anyhow::bail!("unsupported platform")
}
#[cfg(not(target_os = "linux"))]
fn terminate_session(_: &dyn Child) {}

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}
