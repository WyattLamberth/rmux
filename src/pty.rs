//! Unix PTY adapter. All reads/writes are nonblocking and budgeted per tick.
use crate::{
    layout::Rect,
    session::{MAX_INPUT, OUTPUT_CHUNK},
};
use anyhow::{Context, Result, bail};
use nix::{
    fcntl::{FcntlArg, OFlag, fcntl},
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
};

pub const QUEUE_BYTES: usize = 64 * 1024;
pub const READS_PER_TICK: usize = 4;
pub const WRITES_PER_TICK: usize = 4;

pub struct Tick {
    pub output: Vec<Vec<u8>>,
    pub exit: Option<u32>,
}

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    pending: VecDeque<u8>,
    eof: bool,
    exited: Option<u32>,
    stopped: bool,
}

fn size(rect: Rect) -> PtySize {
    PtySize {
        rows: rect.rows.max(1),
        cols: rect.cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl Pty {
    pub fn spawn(mut command: CommandBuilder, rect: Rect) -> Result<Self> {
        command.env("TERM", "xterm-256color");
        let pair = native_pty_system()
            .openpty(size(rect))
            .context("open PTY")?;
        let fd = pair
            .master
            .as_raw_fd()
            .context("Unix PTY descriptor unavailable")?;
        // nix 0.29 exposes a safe RawFd API; master owns the fd throughout this call.
        let flags = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL)?);
        fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        // Allocate all fallible resources before spawning, avoiding an orphan on error.
        let child = pair.slave.spawn_command(command).context("spawn shell")?;
        drop(pair.slave);
        Ok(Self {
            master: pair.master,
            reader,
            writer,
            child,
            pending: VecDeque::with_capacity(QUEUE_BYTES),
            eof: false,
            exited: None,
            stopped: false,
        })
    }

    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }
    pub fn resize(&self, rect: Rect) -> Result<()> {
        self.master.resize(size(rect))
    }

    /// Admission is atomic. Backpressure rejects input without losing its tail.
    pub fn enqueue(&mut self, bytes: &[u8]) -> bool {
        if self.stopped
            || self.exited.is_some()
            || bytes.len() > MAX_INPUT
            || self.pending.len() + bytes.len() > QUEUE_BYTES
        {
            return false;
        }
        self.pending.extend(bytes);
        true
    }

    pub fn pending_bytes(&self) -> usize {
        self.pending.len()
    }

    pub fn tick(&mut self) -> Result<Tick> {
        let mut output = Vec::with_capacity(READS_PER_TICK);
        if self.stopped {
            return Ok(Tick {
                output,
                exit: self.exited,
            });
        }
        for _ in 0..WRITES_PER_TICK {
            if self.pending.is_empty() || self.exited.is_some() {
                break;
            }
            let (front, _) = self.pending.as_slices();
            match self.writer.write(&front[..front.len().min(OUTPUT_CHUNK)]) {
                Ok(0) => bail!("PTY write returned zero"),
                Ok(n) => {
                    self.pending.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e).context("write PTY"),
            }
        }
        for _ in 0..READS_PER_TICK {
            if self.eof {
                break;
            }
            let mut buffer = [0; OUTPUT_CHUNK];
            match self.reader.read(&mut buffer) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(n) => output.push(buffer[..n].to_vec()),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // Linux reports EIO when the slave closes.
                Err(e) if e.raw_os_error() == Some(nix::libc::EIO) => {
                    self.eof = true;
                    break;
                }
                Err(e) => return Err(e).context("read PTY"),
            }
        }
        if self.exited.is_none() {
            self.exited = self.child.try_wait()?.map(|s| s.exit_code());
        }
        // Do not throw away buffered final output when wait wins the race with reads.
        let exit = if self.exited.is_some() && (self.eof || output.is_empty()) {
            self.exited
        } else {
            None
        };
        Ok(Tick { output, exit })
    }

    /// Kill the foreground job group and shell group, then reap the owned child.
    pub fn stop(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        self.pending.clear();
        let foreground = self.master.process_group_leader();
        let shell = self
            .child
            .process_id()
            .and_then(|id| i32::try_from(id).ok())
            .map(Pid::from_raw);
        let mut first_error = None;
        for group in [foreground.map(Pid::from_raw), shell].into_iter().flatten() {
            if group.as_raw() <= 0 {
                continue;
            }
            if let Err(e) = killpg(group, Signal::SIGKILL) {
                if e != nix::errno::Errno::ESRCH {
                    first_error.get_or_insert(e);
                }
            }
        }
        if let Some(e) = first_error {
            return Err(e).context("terminate PTY process group");
        }
        if self.exited.is_none() {
            self.exited = Some(self.child.wait()?.exit_code());
        }
        self.stopped = true;
        Ok(())
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
