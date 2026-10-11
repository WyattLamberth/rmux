#![forbid(unsafe_code)]
//! Synthetic, release-mode baseline. No shell or user terminal content is recorded.
use anyhow::{Result, bail};
use portable_pty::CommandBuilder;
use rmux::{
    layout::Axis,
    session::{Event, OUTPUT_CHUNK, PaneId, Session},
    terminal::Terminal,
    ui,
};
use rmux::{layout::Rect, pty::Pty};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn samples(name: &str, count: usize, mut work: impl FnMut() -> Result<()>) -> Result<()> {
    for _ in 0..10 {
        work()?;
    }
    let mut times = Vec::with_capacity(count);
    for _ in 0..count {
        let start = Instant::now();
        work()?;
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / count as f64;
    println!(
        "latency,{name},samples={count},median_ms={:.4},p95_ms={:.4},mean_ms={mean:.4}",
        times[count / 2],
        times[(count * 95 / 100).min(count - 1)]
    );
    Ok(())
}

fn main() -> Result<()> {
    println!(
        "rmux synthetic release benchmark; latency includes allocation unless stated; RSS is Linux process RSS"
    );
    for (name, bytes) in [
        (
            "plain",
            b"synthetic output line\r\n".repeat(OUTPUT_CHUNK / 23),
        ),
        (
            "ansi_unicode",
            "\x1b[31m界e\u{301} output\x1b[0m\r\n"
                .as_bytes()
                .repeat(128),
        ),
    ] {
        let bytes = &bytes[..bytes.len().min(OUTPUT_CHUNK)];
        let mut terminal = Terminal::new(24, 80, 1000);
        let total = bytes.len() * 16_384;
        let start = Instant::now();
        for _ in 0..16_384 {
            terminal.process(black_box(bytes));
        }
        let elapsed = start.elapsed().as_secs_f64();
        let first_rss = rss_kib();
        for _ in 0..16_384 {
            terminal.process(black_box(bytes));
        }
        println!(
            "parser,{name},bytes={total},mib_per_sec={:.2},rss_after_first_kib={first_rss:?},rss_after_second_kib={:?}",
            total as f64 / 1_048_576.0 / elapsed,
            rss_kib()
        );
        black_box(terminal.screen());
    }
    {
        let mut terminal = Terminal::new(24, 80, 1000);
        terminal.process(b"\x1b]");
        let chunk = [b'x'; OUTPUT_CHUNK];
        let before = rss_kib();
        let start = Instant::now();
        for _ in 0..32_768 {
            terminal.process(black_box(&chunk));
        }
        println!(
            "parser,unterminated_osc,bytes={},mib_per_sec={:.2},rss_before_kib={before:?},rss_after_kib={:?}",
            chunk.len() * 32_768,
            128.0 / start.elapsed().as_secs_f64(),
            rss_kib()
        );
    }
    for (cols, rows) in [(80, 24), (500, 201)] {
        for count in [1, 4, 16] {
            let (mut session, _) = Session::new(cols, rows);
            session.apply(Event::Spawned(PaneId(0)));
            for id in 1..count {
                session.apply(Event::Split(if id % 2 == 0 {
                    Axis::Rows
                } else {
                    Axis::Columns
                }));
                session.apply(Event::Spawned(PaneId(id)));
            }
            // Exercise colored, wide and combining cells rather than blank-only frames.
            for id in 0..count {
                for _ in 0..16 {
                    session.apply(Event::Output(
                        PaneId(id),
                        "\x1b[32m界e\u{301} synthetic output\x1b[0m\r\n"
                            .repeat(100)
                            .into_bytes(),
                    ));
                }
            }
            let mut frame = Vec::new();
            let name = format!("render_{cols}x{rows}_{count}_panes");
            samples(&name, 200, || {
                frame.clear();
                ui::render(&mut frame, &session, cols, rows, "benchmark")?;
                black_box(&frame);
                Ok(())
            })?;
            println!(
                "frame,{name},bytes={},retained_capacity={},rss_kib={:?}",
                frame.len(),
                frame.capacity(),
                rss_kib()
            );
            samples(&format!("resize_{cols}x{rows}_{count}_panes"), 200, || {
                black_box(session.apply(Event::Resize {
                    cols: cols - 1,
                    rows: rows - 1,
                }));
                black_box(session.apply(Event::Resize { cols, rows }));
                Ok(())
            })?;
        }
    }
    let fd_before = std::fs::read_dir("/proc/self/fd")?.count();
    samples("pty_spawn_exit_reap", 50, || {
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "exit 0"]);
        let mut pty = Pty::spawn(
            command,
            Rect {
                cols: 80,
                rows: 24,
                ..Rect::default()
            },
        )?;
        drain(&mut pty)?;
        Ok(())
    })?;
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "head -c 16777216 /dev/zero"]);
    let start = Instant::now();
    let mut pty = Pty::spawn(
        command,
        Rect {
            cols: 80,
            rows: 24,
            ..Rect::default()
        },
    )?;
    let bytes = drain(&mut pty)?;
    let elapsed = start.elapsed().as_secs_f64();
    drop(pty);
    let fd_after = std::fs::read_dir("/proc/self/fd")?.count();
    println!(
        "pty,sustained_output,bytes={bytes},mib_per_sec={:.2},fd_before={fd_before},fd_after={fd_after}",
        bytes as f64 / 1_048_576.0 / elapsed
    );
    anyhow::ensure!(
        bytes == 16 * 1024 * 1024 && fd_after == fd_before,
        "PTY benchmark output/descriptor mismatch"
    );
    Ok(())
}

fn drain(pty: &mut Pty) -> Result<usize> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut total = 0;
    while Instant::now() < deadline {
        let tick = pty.tick()?;
        total += tick.output.iter().map(Vec::len).sum::<usize>();
        if tick.exit.is_some() {
            return Ok(total);
        }
        std::thread::yield_now();
    }
    bail!("PTY benchmark deadline exceeded")
}
