# rmux delivery plan

Owner: WyattLamberth (product direction). Implementation: Codex.
Updated: 2026-10-11. No human code review is implied.

GitHub holds public engineering issues and validation-linked PRs. Linear holds delivery planning, priorities, milestones, dependencies and
progress in the connected [rmux project](https://linear.app/rmux/project/rmux-a4e456a8b228). Keep both linked; do not mark work Done until
its PR is merged and its acceptance checks pass. Dates are estimates only when
explicitly agreed, not automatic deadlines.

## Milestones and dependencies

| Milestone | GitHub issue | Priority | Depends on | Current state / acceptance gate |
| --- | --- | --- | --- | --- |
| Attached alpha | [#1 runtime](https://github.com/WyattLamberth/rmux/issues/1) | High | None | In progress: shell PTYs, split tree, input, scrollback, cleanup; real PTY tests and terminal restoration required |
| Reproducible foundation | [#2 seeded testing](https://github.com/WyattLamberth/rmux/issues/2) | High | #1 transitions | Backlog: replayable fault traces through production state machine |
| Reproducible foundation | [#3 pinned Nix](https://github.com/WyattLamberth/rmux/issues/3) | High | #1 Cargo package | Backlog: initial flake/lock prepared; Nix checks and package execution unverified |
| Reproducible foundation | [#4 CI/releases](https://github.com/WyattLamberth/rmux/issues/4) | High | #2, #3 | Backlog: immutable action pins, least privilege, all quality gates and tag artifacts |
| Attached alpha | [#7 compatibility](https://github.com/WyattLamberth/rmux/issues/7) | High | #1 | Backlog: terminal queries, modes, signals, process groups, slow consumers and sustained output |
| Observable alpha | [#5 metrics](https://github.com/WyattLamberth/rmux/issues/5) | Normal | #1, #4 | Backlog: opt-in bounded aggregate metrics, loopback and provisioned dashboard |
| Persistent beta | [#6 detach/reattach](https://github.com/WyattLamberth/rmux/issues/6) | Normal | #2, #4, #7 | Backlog: protocol/ownership/resize policy agreed before server implementation |

## Working cadence

Take one bounded issue or subtask at a time. Record acceptance criteria and
blocking dependencies before implementation. Move to In Progress when work
starts, In Review when a PR exists, and Done after merge plus validation. Keep
failed checks and unresolved defects visible instead of representing partial
work as complete. Record scope decisions and meaningful progress in project
updates. Assign product decisions to the owner and implementation to Codex;
never invent an assignee or approval.

## Linear tracking

The rmux project and four milestones are created. Seven linked issues are
RMU-5 (GitHub #1), RMU-6 (#2), RMU-7 (#3), RMU-8 (#4), RMU-9 (#5),
RMU-10 (#6) and RMU-11 (#7). Native priority, milestone and blocking
relationships are set. Runtime is In Progress pending its PR; the rest remain
in Backlog. No work is marked Done before merge and acceptance checks.

The owner has requested Scrum principles: use timeboxed Linear cycles as
sprints, a product goal and sprint goal, refined sprint-sized backlog items,
relative estimates, daily progress/blocker inspection, a sprint review of the
working increment, and a retrospective with an actionable improvement. Use two-day sprints of focused agentic work; estimates describe complexity,
not human engineering hours, and scope follows actual agent throughput. Enable native cycle graphs and
project progress; configure Insights dashboards where the workspace plan
supports them. Never fabricate velocity, historical completion or sprint data.

The [Scrum working agreement](https://linear.app/rmux/document/scrum-working-agreement-and-reporting-52daab5378bb) records sprint events, readiness and Done criteria. Native cycle/chart setup awaits browser sign-in; no completed velocity is claimed.
