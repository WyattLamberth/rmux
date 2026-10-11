# Project workflow

All implementation is AI-generated under the owner's product direction.
Use the local Git identity `Codex <codex@openai.com>` for commits. Include a
`Co-authored-by: Codex <codex@openai.com>` trailer when another author identity
is necessary. Never claim a human reviewed generated code.

Track planned work and unresolved defects in the public GitHub issue queue.
Use issue-linked branches and pull requests with accurate validation results.
Do not publish secrets, machine credentials, shell transcripts, or private data.

Keep the session state machine deterministic: real PTYs, clocks, signals,
networking, and scheduling belong in adapters. Simulated failures must go
through production state transitions. A failure report must include the seed,
step, and enough trace information to replay it.

Forbid unsafe code. Use ownership and enums to model lifecycle transitions;
avoid panics in runtime paths. Bound queues, retained history, and per-tick work.
Test lifecycle/resource cleanup, failures, small terminal sizes, and sustained
output. Run formatting, Clippy with warnings denied, and relevant tests.

Use Herdr for coordinated parallel work when requested. Assign exclusive file
ownership to prevent conflicting edits. Keep user focus unchanged. Do not answer
another agent's approval dialog without the owner's authorization.

Metrics are opt-in, aggregate, and contain no terminal or command content.
Bind to loopback by default. Reproducible builds require both Cargo.lock and
flake.lock. CI/release credentials get the minimum necessary permissions.
