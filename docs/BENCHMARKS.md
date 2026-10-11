# Runtime performance baseline

Measured 2026-10-11 on local x86_64 Linux, Rust 1.96.1, release profile with thin LTO. Three sequential full runs; no concurrent benchmark workers. These are local measurements, not portable performance guarantees.

Reproduce with `cargo bench --locked --bench runtime`. Linux `/proc`, `/dev/ptmx`, `/bin/sh`, `head`, and `/dev/zero` are required. All payloads are synthetic. No user shell input/output or machine credentials are retained.

## Throughput and retained memory

| Workload | Median MiB/s | Observed range | Memory/resource observation |
| --- | ---: | ---: | --- |
| Plain parsing | 47.42 | 46.91–48.59 | RSS unchanged between repeated batches in all runs |
| ANSI/Unicode parsing | 45.51 | 45.43–45.62 | RSS unchanged between repeated batches in all runs |
| Discard unterminated OSC | 1549.37 | 1528.60–1607.34 | RSS unchanged between repeated batches in all runs |
| Raw PTY transfer (16 MiB) | 637.98 | 608.30–683.72 | FD count 4 before and 4 after each run |

Plain parsing processes 67,076,096 bytes twice at 24×80 with 1,000 retained history lines. ANSI/Unicode parsing processes 50,331,648 bytes twice at the same size. The OSC case discards 128 MiB without a terminator, with identical before/after RSS in each run. PTY throughput measures the nonblocking adapter and kernel transport without parsing or outer-terminal rendering; its value must not be presented as end-to-end UI throughput.

## Latency

Each render/resize case has 10 warmups and 200 timed samples per run (600 total). Spawn/exit/reap has 10 warmups and 50 timed samples per run (150 total). The median column is the median of the three run medians; the p95 range gives each run’s p95, not a pooled percentile.

| Operation | Median ms | Run p95 range, ms |
| --- | ---: | ---: |
| render_80x24_1_panes | 0.0423 | 0.0434–0.0450 |
| resize_80x24_1_panes | 0.0005 | 0.0005–0.0008 |
| render_80x24_4_panes | 0.0446 | 0.0455–0.0490 |
| resize_80x24_4_panes | 0.0005 | 0.0006–0.0006 |
| render_80x24_16_panes | 0.0442 | 0.0462–0.0464 |
| resize_80x24_16_panes | 0.0008 | 0.0008–0.0008 |
| render_500x201_1_panes | 2.0982 | 2.1103–2.1813 |
| resize_500x201_1_panes | 0.0027 | 0.0027–0.0027 |
| render_500x201_4_panes | 2.1459 | 2.1587–2.2196 |
| resize_500x201_4_panes | 0.0031 | 0.0032–0.0036 |
| render_500x201_16_panes | 2.1675 | 2.1696–2.4647 |
| resize_500x201_16_panes | 0.0034 | 0.0034–0.0036 |
| pty_spawn_exit_reap | 1.2630 | 1.4641–1.5767 |

Rendering includes cell inspection, colors/attributes, frame encoding and writes to a retained in-memory Vec. It excludes stdout flushing to a real terminal, transport latency and the display emulator. 80×24 and 500×201 include one status row; the maximum content area is 500×200. Nested 16-pane layouts intentionally include tiny/hidden panes. Resize samples are a one-cell shrink/restore pair, including parser resize and session effects, and exclude OS PTY ioctls. PTY samples include allocation, spawn, observed exit, owned-child reaping and handle cleanup.

## Defect found and correctness tradeoff

The initial vt100 0.16.2 benchmark panicked while filling the 80×24, 16-pane case. Reproduction is deterministic: create a session, alternately split columns/rows until 16 panes, mark each spawned, and feed each pane 16 chunks of the benchmark’s colored wide/combining text. One-row wrapping subtracts a scrolled row from zero, causing an invalid row and an unwrap panic; one-column wide glyph handling also needs protection. This is a scripted benchmark, not a seeded simulation.

The runtime now uses atuin-vt100 0.19.1, which handles those edges and models dimensions with NonZeroU16. A regression test exercises 1–3 rows/columns, ASCII/wide/combining output, alternate-screen switching and shrink/grow; the complete benchmark passes.

The buggy parser’s first partial run measured 104.45 MiB/s plain and 116.73 MiB/s ANSI/Unicode. The corrected parser’s medians are lower (47.42 and 45.51 MiB/s). This preliminary before/after comparison is not a paired statistical experiment; the old version could not complete the workload. Correctness wins here, and future parser optimization must retain the regression coverage.

## Limits and next measurements

RSS is coarse process memory from Linux /proc, not allocation accounting or a proof of a universal bound. Identical repeated-batch RSS supports this workload’s plateau only. Results depend on hardware, compiler, scheduling, allocator and dependency versions. Stable CI budgets, application-level latency under slow outer terminals, foreground/background process-group hardening and longer duration workloads remain work in GitHub #12/#7. Do not claim velocity or performance improvements without measurements.

Benchmark source SHA256: `199a01ae9fde1b5d914f12321b99f5192c7fdf4bc59d937f38d95be62e5d731c`.
