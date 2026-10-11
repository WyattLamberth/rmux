# rmux

A terminal multiplexer written in safe, idiomatic Rust. This project is being
implemented entirely by **OpenAI Codex**, with product direction from
[@WyattLamberth](https://github.com/WyattLamberth). AI writes the code, tests,
documentation, and infrastructure and manages the GitHub work queue. Commits
are attributed to `Codex <codex@openai.com>`. Human guidance does not imply a
human code review. This is an independent project, not an OpenAI product.

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
