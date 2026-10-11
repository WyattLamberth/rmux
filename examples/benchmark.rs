//! Repeatable benchmark of production domain/application transitions.
//!
//! Run with `cargo run --release --locked --example benchmark`.
//! RMUX_BENCH_ITERATIONS accepts 1..=10000000 decimal iterations (default 1000000).
//! Each cycle dispatches seven events; Controller also handles one Spawned event.
//! Allocation and layout work are included. The host performs no I/O, so these
//! results make no claim about PTY throughput, rendering, networking, or latency.
#![forbid(unsafe_code)]

use rmux_core::{Axis, Controller, Event, PaneHost, PaneId, Rect, Session};
use std::{convert::Infallible, hint::black_box, process::ExitCode, time::Instant};

const DEFAULT_ITERATIONS: u64 = 1_000_000;
const MAX_ITERATIONS: u64 = 10_000_000;
const WARMUP_ITERATIONS: u64 = 10_000;
const EVENTS_PER_CYCLE: u64 = 7;
const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 160,
    height: 80,
};

struct NoopHost;

impl PaneHost for NoopHost {
    type Error = Infallible;

    fn spawn(&mut self, id: PaneId, rows: u16, cols: u16) -> Result<(), Self::Error> {
        black_box((id, rows, cols));
        Ok(())
    }

    fn close(&mut self, id: PaneId) -> Result<(), Self::Error> {
        black_box(id);
        Ok(())
    }

    fn resize(&mut self, id: PaneId, rows: u16, cols: u16) -> Result<(), Self::Error> {
        black_box((id, rows, cols));
        Ok(())
    }

    fn write(&mut self, id: PaneId, bytes: &[u8]) -> Result<(), Self::Error> {
        black_box((id, bytes));
        Ok(())
    }
}

fn iterations() -> Result<u64, String> {
    let value = match std::env::var("RMUX_BENCH_ITERATIONS") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return Ok(DEFAULT_ITERATIONS),
        Err(error) => return Err(format!("RMUX_BENCH_ITERATIONS: {error}")),
    };
    let valid_decimal = !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    let parsed = value.parse::<u64>().ok();
    if valid_decimal && let Some(count @ 1..=MAX_ITERATIONS) = parsed {
        Ok(count)
    } else {
        Err(format!(
            "RMUX_BENCH_ITERATIONS must be a decimal integer in 1..={MAX_ITERATIONS}"
        ))
    }
}

fn cycle(controller: &mut Controller<NoopHost>, index: u64) {
    let axis = if index.is_multiple_of(2) {
        Axis::Columns
    } else {
        Axis::Rows
    };
    controller.dispatch(black_box(Event::Split(axis)));
    let split_pane = black_box(controller.session.focused())
        .expect("a successful split must focus its new pane");
    controller.dispatch(black_box(Event::FocusPrevious));
    controller.dispatch(black_box(Event::FocusNext));
    controller.dispatch(black_box(Event::Zoom));
    controller.dispatch(black_box(Event::Write(b"benchmark input\n".to_vec())));
    controller.dispatch(black_box(Event::Zoom));
    controller.dispatch(black_box(Event::Close(split_pane)));
    black_box(&controller.session);
}

fn assert_final_invariant(controller: &Controller<NoopHost>) {
    assert_eq!(controller.session.pane_ids(), [1]);
    assert_eq!(controller.session.focused(), Some(1));
    assert!(!controller.session.is_zoomed());
    assert_eq!(controller.session.rectangles(), [(1, AREA)]);
    assert!(controller.notices.is_empty());
}

fn main() -> ExitCode {
    let count = match iterations() {
        Ok(count) => count,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    // The initial pane is conceptually bootstrapped by the no-op adapter.
    let mut warmup = Controller::new(Session::new(AREA), NoopHost);
    for index in 0..WARMUP_ITERATIONS {
        cycle(black_box(&mut warmup), black_box(index));
    }
    assert_final_invariant(&warmup);

    let mut controller = Controller::new(Session::new(AREA), NoopHost);
    let start = Instant::now();
    for index in 0..count {
        cycle(black_box(&mut controller), black_box(index));
    }
    let elapsed_seconds = start.elapsed().as_secs_f64();
    assert_final_invariant(&controller);
    let ns_per_cycle = elapsed_seconds * 1_000_000_000.0 / count as f64;
    println!(
        "{{\"elapsed_seconds\":{elapsed_seconds:.9},\"iterations\":{count},\"ns_per_cycle\":{ns_per_cycle:.3},\"events_per_cycle\":{EVENTS_PER_CYCLE},\"internal_spawned_events_per_cycle\":1,\"scope\":\"core controller with no-op host; no PTY or network performance claim\"}}"
    );
    ExitCode::SUCCESS
}
