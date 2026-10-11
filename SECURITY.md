# Security boundaries

rmux is an early-development local terminal application. All implementation and
testing are AI-generated; daily-use hardening is ongoing.

## Current trust model

Shells execute as the invoking user with that user's environment and privileges.
This application is not a sandbox for untrusted commands. It does not need root
or elevated capabilities. Never run it as a privileged daemon to share shells
between users.

Child output is parsed into terminal cells. The renderer creates its own escape
sequences instead of forwarding arbitrary child output to the host terminal.
Clipboard and host-title escape requests are not forwarded. Terminal input/output
and commands are not exported through telemetry.

Metrics are disabled unless `RMUX_METRICS_ADDR` is set. Prefer
`127.0.0.1:9464`. The endpoint is plaintext and unauthenticated; explicitly binding
another address exposes aggregate usage information to that network. The local
Grafana/Prometheus setup must stay on loopback. Request buffers, timeouts, and
queues are bounded. These bounds do not provide complete resource isolation.

We forbid unsafe Rust in our crates. Dependencies implementing operating-system
interfaces can use unsafe code. CI checks dependency advisories and pinned build
inputs; those checks do not prove the absence of vulnerabilities.

## Release and infrastructure boundaries

GitHub Actions use immutable action revisions, minimal permissions, and no
external coverage-upload service. Release artifacts require quality gates.
Pull-request checks must not receive deployment secrets or execute under
`pull_request_target`. Cargo and Nix dependencies are locked. Updates are tracked
and reviewed through PRs, with Codex attribution preserved.

## Before remote sessions and daily use

Remote access will use SSH transport and normal host-key verification, with a
versioned, bounded protocol and per-user local socket access. Untrusted hosts
must not inject local commands, paths, clipboard contents, or configuration.
Credentials and known-host decisions remain with SSH rather than custom crypto.

Remaining security work includes process-group cleanup, hostile terminal-output
and screen-size budgets, dependency review, secure socket creation/replacement,
protocol fuzzing, reconnect fault tests, configuration trust boundaries, and
release integrity verification. Track these in the public issue queue without
publishing exploit credentials, shell transcripts, or private keys.

Use GitHub's private vulnerability reporting if enabled to report a sensitive
finding. Otherwise contact the repository owner privately; do not place secrets
in a public issue.
