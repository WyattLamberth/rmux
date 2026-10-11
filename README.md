# rmux

A terminal multiplexer written in safe, idiomatic Rust. AI agents implement
this project under product direction from
[@WyattLamberth](https://github.com/WyattLamberth). Any agent may write the
code, tests, documentation, and infrastructure. This is not a ChatGPT-only
effort. Human guidance does not imply a human code review. This is an
independent project, not an OpenAI product.

## Status

Early development. The first milestone is an attached terminal multiplexer:
real shell PTYs, nested splits, focus switching, zoom, scrollback, and terminal
resize handling. Persistent server sessions and detach/reattach are subsequent
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

## Try it on Linux

```sh
cargo run --locked -- --shell /bin/bash
# Or with the pinned Nix environment:
nix run
```

Press `Ctrl-b`, release it, then a command:

| Key | Action |
| --- | --- |
| `%` / `"` | Split into columns / rows |
| `o` or `Tab` / `p` | Next / previous pane |
| `z` | Zoom focused pane |
| `PageUp` / `PageDown` | Scroll history up / down |
| `x` / `q` | Close focused shell / quit all shells; `y` confirms |
| `?` | Help |
| `Ctrl-b` | Send a literal Ctrl-b to the shell |
| `Esc` | Cancel prefix |

New shells start in the directory where rmux was launched. A split currently
inherits that launch directory, rather than the focused shell's current directory.
Exiting the last shell closes rmux. Closing rmux terminates its shells; sessions
do not yet survive a client disconnect. It has no mouse or copy-mode selection yet.

## Inspect the work

- [Issues](https://github.com/WyattLamberth/rmux/issues) and
  [pull requests](https://github.com/WyattLamberth/rmux/pulls).
- [CI runs](https://github.com/WyattLamberth/rmux/actions) include quality checks,
  deterministic seed replay, Nix builds, and dependency advisories.
- [Architecture](docs/architecture.md), [security boundaries](SECURITY.md),
  [local telemetry](observability/README.md), and [engineering scripts](scripts/README.md).

```sh
cargo test --workspace --locked
RMUX_TEST_SEED=42 RMUX_TEST_STEPS=100000 cargo test --locked --test universe -- --nocapture
cargo run --release --locked --example benchmark
RMUX_METRICS_ADDR=127.0.0.1:9464 cargo run --locked
```

The benchmark measures core transitions with a no-op shell adapter. It does not
measure rendering speed, PTY responsiveness, SSH latency, or real shell startup.
The seed simulator tests session decisions and application port failures; real
PTY, HTTP, and terminal tests validate adapters separately. These results are a
foundation, not a claim of complete deterministic coverage of the whole program.
