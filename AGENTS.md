# Project workflow

The owner grants Codex broad creative freedom for rmux. Make independent
product, design, architecture, and implementation choices within the owner's
direction; explore useful improvements and keep decisions concrete and
reviewable. Do not be afraid of aggressive refactoring when it improves correctness,
architecture, usability, or performance. Engineering time is not a deciding
cost under the owner's direction; prefer the stronger result over preserving
an inferior design to save implementation effort. Benchmark aggressively to
quantify results: record reproducible workloads, throughput, latency, memory
and resource behavior where relevant; compare before/after for performance
claims. Keep measurements honest and limitations explicit. This does not waive
safety, privacy, validation, or honest provenance.

All implementation is AI-generated under the owner's product direction.
Use the local Git identity `Codex <codex@openai.com>` for commits. Include a
`Co-authored-by: Codex <codex@openai.com>` trailer when another author identity
is necessary. Never claim a human reviewed generated code.

Track planned work and unresolved defects in the public GitHub issue queue.
Use issue-linked branches and pull requests with accurate validation results.
Also maintain delivery planning in the owner's connected Linear workspace:
link GitHub issues/PRs, track milestones, priorities, dependencies, and status,
and mark Done only after merge and required validation. Use Scrum principles:
two-day sprints of focused agentic work, sprint goals, refined backlog, Linear
cycles, complexity estimates (not human-hour estimates), daily inspection,
sprint reviews and retrospectives; keep native charts backed by actual work.
See docs/PLAN.md.
If Linear is not connected, keep the plan current locally until it is available.
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
