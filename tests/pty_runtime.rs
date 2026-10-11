#![forbid(unsafe_code)]
use anyhow::{Result, bail};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use portable_pty::CommandBuilder;
use rmux::{
    layout::Rect,
    pty::{Pty, QUEUE_BYTES, READS_PER_TICK},
    session::{MAX_INPUT, OUTPUT_CHUNK},
};
use std::{
    thread,
    time::{Duration, Instant},
};

static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn rect(rows: u16, cols: u16) -> Rect {
    Rect {
        rows,
        cols,
        ..Rect::default()
    }
}
fn shell(script: &str) -> CommandBuilder {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", script]);
    command
}
fn collect(pty: &mut Pty, until: impl Fn(&[u8], Option<u32>) -> bool) -> Result<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut bytes = Vec::new();
    while Instant::now() < deadline {
        let tick = pty.tick()?;
        assert!(tick.output.len() <= READS_PER_TICK);
        for chunk in tick.output {
            assert!(chunk.len() <= OUTPUT_CHUNK);
            bytes.extend(chunk);
        }
        if until(&bytes, tick.exit) {
            return Ok(bytes);
        }
        thread::sleep(Duration::from_millis(2));
    }
    bail!("PTY test exceeded deadline ({} output bytes)", bytes.len())
}
fn has(bytes: &[u8], text: &str) -> bool {
    String::from_utf8_lossy(bytes).contains(text)
}
fn gone(pid: u32) -> bool {
    i32::try_from(pid)
        .is_ok_and(|pid| kill(Pid::from_raw(pid), None) == Err(nix::errno::Errno::ESRCH))
}

#[test]
fn real_pty_lifecycle_output_resize_backpressure_and_fd_cleanup() -> Result<()> {
    let _guard = TEST_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("PTY test lock poisoned"))?;
    let before = std::fs::read_dir("/proc/self/fd")?.count();
    {
        let mut pty = Pty::spawn(
            shell(
                "test -t 0 && test -t 1 || exit 9; stty size; read line; printf 'got:%s\\n' \"$line\"; stty size",
            ),
            rect(7, 19),
        )?;
        let start = collect(&mut pty, |bytes, _| has(bytes, "7 19"))?;
        assert!(has(&start, "7 19"));
        pty.resize(rect(3, 11))?;
        assert!(pty.enqueue(b"hello\n"));
        let end = collect(&mut pty, |_, exit| exit.is_some())?;
        assert!(has(&end, "got:hello"));
        assert!(has(&end, "3 11"));
    }
    {
        let mut pty = Pty::spawn(
            shell(
                "i=0; while [ $i -lt 10000 ]; do printf 'line-%s\\n' \"$i\"; i=$((i+1)); done; printf 'FINAL\\n'",
            ),
            rect(2, 2),
        )?;
        let bytes = collect(&mut pty, |_, exit| exit.is_some())?;
        assert!(has(&bytes, "line-9999"));
        assert!(has(&bytes, "FINAL"));
    }
    {
        let mut pty = Pty::spawn(
            shell("trap '' HUP; stty -echo; printf READY; sleep 60"),
            rect(2, 2),
        )?;
        collect(&mut pty, |bytes, _| has(bytes, "READY"))?;
        let pid = pty
            .process_id()
            .ok_or_else(|| anyhow::anyhow!("missing pid"))?;
        for _ in 0..QUEUE_BYTES / MAX_INPUT {
            assert!(pty.enqueue(&[b'x'; MAX_INPUT]));
        }
        assert_eq!(pty.pending_bytes(), QUEUE_BYTES);
        assert!(!pty.enqueue(b"x"));
        let start = Instant::now();
        for _ in 0..100 {
            pty.tick()?;
        }
        assert!(start.elapsed() < Duration::from_secs(1));
        pty.stop()?;
        pty.stop()?;
        assert!(gone(pid));
    }
    {
        let mut pty = Pty::spawn(shell("printf READY; sleep 60"), rect(2, 2))?;
        collect(&mut pty, |bytes, _| has(bytes, "READY"))?;
        let pid = pty
            .process_id()
            .ok_or_else(|| anyhow::anyhow!("missing pid"))?;
        drop(pty);
        assert!(gone(pid));
    }
    assert!(
        Pty::spawn(
            CommandBuilder::new("/definitely/missing/rmux-shell"),
            rect(1, 1)
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir("/proc/self/fd")?.count(), before);
    Ok(())
}

#[test]
fn attached_runtime_accepts_commands_and_exits_cleanly() -> Result<()> {
    let _guard = TEST_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("PTY test lock poisoned"))?;
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_rmux"));
    command.args([
        "--",
        "/bin/sh",
        "-c",
        "printf 'RMUX-READY\\n'; read value; printf 'VALUE:%s\\n' \"$value\"; sleep 60",
    ]);
    let mut pty = Pty::spawn(command, rect(12, 60))?;
    collect(&mut pty, |bytes, _| has(bytes, "RMUX-READY"))?;
    assert!(pty.enqueue(b"hello\r"));
    collect(&mut pty, |bytes, _| has(bytes, "VALUE:hello"))?;
    // A real prefix split spawns a second shell. Focus cycles, zoom resizes.
    assert!(pty.enqueue(b"\x02%"));
    collect(&mut pty, |bytes, _| has(bytes, "pane 1"))?;
    assert!(pty.enqueue(b"\x02z\x02o"));
    collect(&mut pty, |bytes, _| has(bytes, "pane 0"))?;
    pty.resize(rect(1, 1))?;
    thread::sleep(Duration::from_millis(30));
    pty.resize(rect(12, 60))?;
    assert!(pty.enqueue(b"\x02q"));
    let bytes = collect(&mut pty, |_, exit| exit.is_some())?;
    assert!(has(&bytes, "\x1b[?1049l")); // Alternate screen restored.
    assert!(has(&bytes, "\x1b[?2004l")); // Bracketed paste disabled.
    Ok(())
}

#[test]
fn attached_runtime_restores_terminal_on_spawn_failure_and_signal() -> Result<()> {
    let _guard = TEST_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("PTY test lock poisoned"))?;
    for fail in [true, false] {
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_rmux"));
        command.args(if fail {
            vec!["--", "/definitely/missing/rmux-shell"]
        } else {
            vec!["--", "/bin/sh", "-c", "printf SIGNAL-READY; sleep 60"]
        });
        let mut pty = Pty::spawn(command, rect(8, 60))?;
        if !fail {
            collect(&mut pty, |bytes, _| has(bytes, "SIGNAL-READY"))?;
            let pid = pty
                .process_id()
                .ok_or_else(|| anyhow::anyhow!("missing pid"))?;
            kill(Pid::from_raw(i32::try_from(pid)?), Signal::SIGTERM)?;
        }
        let bytes = collect(&mut pty, |_, exit| exit.is_some())?;
        assert!(has(&bytes, "\x1b[?1049l"));
        assert!(has(&bytes, "\x1b[?2004l"));
    }
    Ok(())
}
