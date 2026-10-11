# rmux

A terminal multiplexer written in safe, idiomatic Rust. This project is being
implemented entirely by **OpenAI Codex**, with product direction from
[@WyattLamberth](https://github.com/WyattLamberth). AI writes the code, tests,
documentation, and infrastructure and manages the GitHub work queue. Commits
are attributed to `Codex <codex@openai.com>`. Human guidance does not imply a
human code review. This is an independent project, not an OpenAI product.

## Status

Early development. The attached runtime now provides real shell PTYs, nested
splits, focus switching, zoom, bounded scrollback, and terminal resize handling. Persistent server sessions and detach/reattach are subsequent
milestones, tracked in GitHub issues. Do not treat this as a production tmux
replacement yet.

## Engineering goals

- Forbid unsafe Rust in our crates; use maintained libraries for OS interfaces.
- Keep session decisions independent of real clocks, threads, and processes.
- Replay deterministic simulated universes by seed, exercising faults and
  adversarial scheduling in addition to ordinary unit and real PTY tests.
- Provide a pinned Nix development/build environment and CI release gates.
- Expose opt-in aggregate Prometheus metrics and a provisioned Grafana dashboard,
  without recording shell input, output, working directories, or command names.

GitHub issues are the authoritative backlog. Changes should link to their issue
and record validation and limitations in a pull request.

## Run and controls

On Linux with Rust installed:

```sh
cargo run --release
# Or choose a command (new splits run the same command):
cargo run --release -- -- /bin/bash
```

The default command is `$SHELL`, falling back to `/bin/sh`. Both stdin and stdout
must be terminals. `rmux --help` works without a terminal.

Press **Ctrl-b**, release it, then:

| Key | Action |
| --- | --- |
| `%` | Split into columns |
| `"` | Split into rows |
| `o` | Cycle focus |
| `z` | Toggle focused-pane zoom |
| `[` | Enter scrollback; arrows or Page Up/Down scroll; `q`/Escape returns |
| `x` | Close the focused pane and terminate its shell/foreground job |
| `q` | Quit and terminate all owned shells |
| Ctrl-b | Send a literal Ctrl-b to the shell |

Typing returns the focused pane to live output. Exited panes retain their
screen until closed; rmux exits when all panes have exited or failed. There is
no persistent server or detach command yet.

## Bounds and architecture

`session` and `layout` implement deterministic production transitions, returning
explicit effects. `pty` owns real processes, nonblocking I/O and resize calls;
`main` supplies scheduling and terminal events. `ui` renders parsed cells into
the outer terminal, so child cursor/erase escapes cannot affect neighboring
panes. An RAII guard restores terminal mode on normal exit, errors, SIGTERM,
SIGHUP and SIGINT. SIGKILL cannot run cleanup.

Limits: 16 panes, a 200-row by 500-column content area, 1,000 scrollback lines
per pane, 4 KiB output chunks, four reads and four writes per pane per tick,
32 outer input events per tick, 64 KiB queued input per pane, and 16 KiB per
input/paste event. Oversize pastes and full input queues are rejected with a
status message. Hidden panes retain their last nonzero PTY size. Unsupported
OSC/DCS/APC/PM strings are discarded with constant retained filter state.

The renderer supports basic ANSI colors/attributes, Unicode cells and the
parser's alternate screen. Input supports application cursor mode, common
navigation/function keys, modifiers and bracketed paste. `TERM=xterm-256color`
is a compatibility target, not a claim of complete xterm emulation. Mouse
reporting, terminal query replies, extended keyboard protocols, clipboard/image
escapes and full application-keypad support remain work for
[issue #7](https://github.com/WyattLamberth/rmux/issues/7).

Shutdown kills the shell process group and the PTY foreground process group and
reaps the owned shell. Background jobs in other process groups, daemonized
processes and adversarial session escapes require further lifecycle hardening
in #7; this is not a process sandbox. Linux is the tested platform; other Unix
platforms are not validated and Windows is not supported by this adapter.

## Validation and reproducibility

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Tests include deterministic lifecycle failures, tiny/nested layouts, bounded
history, key/paste encoding, rendering isolation, unterminated control strings,
and real PTYs exercising shell input/output, sustained output, resize, queue
backpressure, direct-child reaping, descriptor cleanup, actual split/focus/zoom
commands, spawn failure and signal restoration. Real PTY tests require Linux
`/proc` and a working `/dev/ptmx`. Tests coordinate descriptor accounting so
concurrent tests cannot skew the cleanup check.

`Cargo.lock`, `rust-toolchain.toml`, `flake.nix` and `flake.lock` pin dependencies
and toolchains. The initial flake targets x86_64-linux and aarch64-linux:
`nix develop`, `nix build`, `nix flake check`. Its sandbox check runs library
unit tests; run the real PTY tests separately outside the sandbox. Nix uses the
Rust toolchain from the locked nixpkgs; direct Cargo uses rust-toolchain.toml.
Nix evaluation/build/package execution and aarch64 execution are **unverified**,
tracked in [issue #3](https://github.com/WyattLamberth/rmux/issues/3). The initial
nixpkgs lock metadata comes from the upstream Nix repository's pinned flake.
Seeded simulation and CI release gates are tracked in #2 and #4.

Delivery milestones and the Linear setup plan are in [docs/PLAN.md](docs/PLAN.md).

## Performance evidence

Run `cargo bench --locked --bench runtime` for the synthetic release baseline.
It covers sustained parsing, unsupported strings, 1/4/16-pane rendering and
resizing, PTY throughput, and spawn/reap/descriptor cleanup. Workloads and
measured results are in [docs/BENCHMARKS.md](docs/BENCHMARKS.md). The baseline
is local evidence, not a portable CI threshold or real-terminal display speed.

Benchmarking exposed a one-row wrapping panic in vt100 0.16.2. The runtime now
uses the maintained atuin-vt100 fork with nonzero terminal dimensions, plus
regression tests for one-row/one-column ASCII, wide/combining text, alternate
screens and resize. The correctness fix has a measured parsing-throughput cost
that is recorded explicitly in the benchmark report.
