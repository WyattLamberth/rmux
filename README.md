# rmux

A terminal multiplexer written in safe, idiomatic Rust. AI agents implement
this project under product direction from
[@WyattLamberth](https://github.com/WyattLamberth). Any agent may write the
code, tests, documentation, and infrastructure. This is not a ChatGPT-only
effort. Human guidance does not imply a human code review. This is an
independent project, not an OpenAI product.

## Status

Early development. Real shell PTYs, windows, nested splits, focus switching,
zoom, scrollback, mouse focus/scroll and terminal resizing are available.
Opt-in local persistence keeps shells across detach/reattach. Remaining terminal,
remote discovery and synchronization acceptance work is tracked in GitHub.

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
cargo run
# Or with the pinned Nix environment:
nix run
```

Configuration is `~/.config/rmux/rmux.toml` (or under `$XDG_CONFIG_HOME`). A missing file uses Rosé Pine dark, a bottom status bar, no pane headers,
zero padding and your `$SHELL`. `rmux --print-default-config` prints them. The file must be owned by
you and must not be group or world writable.

Persistent sessions are off unless you pass `--persist` or set `server.persist`.
Detach with `Ctrl-b d`. The server keeps the shells. Run `rmux --persist` again
to attach to the latest screen. `rmux attach user@host` bootstraps that server
over SSH. `sync.udp_address` is an optional authenticated datagram path; it
must not be an unspecified address.

Press `Ctrl-b`, release it, then a command:

| Key | Action |
| --- | --- |
| `\|` / `-` | Split into columns / rows |
| `h` / `j` / `k` / `l` | Focus left / down / up / right; repeat without prefix for 500ms |
| `c` | Create a window |
| `Ctrl-p` / `Ctrl-n` | Previous / next window |
| `R` | Reload configuration |
| `o` or `Tab` / `p` | Next / previous pane |
| `z` | Zoom focused pane |
| `PageUp` / `PageDown` | Scroll history up / down |
| `x` / `q` | Close focused shell / quit all shells; `y` confirms |
| `d` | Detach, when persistence is enabled |
| `?` | Help |
| `Ctrl-b` | Send a literal Ctrl-b to the shell |
| `Esc` | Cancel prefix |

Splits and new windows inherit the focused shell's current directory on Linux,
falling back to rmux's launch directory if the shell directory cannot be read.
Mouse clicks focus panes and the wheel scrolls history; selection/copy mode and
forwarding mouse events to terminal applications are not implemented. Closing
the last pane in a window returns to another window; closing the last shell exits.
`Ctrl-b q`, then `y`, terminates shells across all windows.

### Configure the interface

The bottom bar shows session name, windows with the current window highlighted,
focused pane, zoom/scroll state, local time and date. `%` and `"` remain split
aliases in the default table. [The owner preset](config/rose-pine.toml) removes
those aliases and translates the supplied tmux controls into native rmux actions.
It is a TOML configuration; rmux does not parse `.tmux.conf`.

```toml
[pane]
header = false
padding_x = 0
padding_y = 0
inherit_cwd = true

[status]
position = "bottom"
left = " #S "
right = " %H:%M %Y-%m-%d "
left_length = 40
right_length = 60
interval_ms = 5000
message_time_ms = 4000
```

`header` controls the per-pane label. Padding is measured in terminal cells on
both sides of each axis, capped at 32 per side. Separators occupy their own
cells; rendering and PTY sizes use the same geometry. Small terminals show only
the focused pane when the full layout cannot fit.

`status.left` supports `#S` (session name) and `##`. `status.right` also supports
strftime clock/date formats. Style colors accept names or `#RRGGBB`. The shipped
colors use [Rosé Pine main](https://rosepinetheme.com/palette/). Window names are
currently `shell`; tmux style directives, activity/bell highlighting, window
renaming, command prompts and copy mode remain follow-up work.

Key tables replace the defaults when supplied. For example, a minimal table:

```toml
[input]
prefix = "C-a"

[keys.prefix]
"|" = "split-columns"
"-" = "split-rows"
h = "focus-left"
j = "focus-down"
k = "focus-up"
l = "focus-right"
R = "reload"
"C-a" = "send-prefix"
q = "quit"
```

Use `[keys.root]` for bindings without a prefix. `Ctrl-` is written `C-`, Alt is
`M-`, and uppercase letters (such as `R`) imply Shift. Help reflects configured
bindings. Reload updates keys, colors, status, headers, padding, mouse and
new-shell settings while keeping live shells. History changes apply to new
panes. Server, synchronization, metrics and resource-limit changes require a
restart. Persistent local clients reload their configuration and ask the server
to reload its own file; remote SSH clients and servers can have distinct files.

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
