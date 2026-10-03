# ADR 0001: Single-process browser with IPC-shaped boundaries

- **Status**: Proposed
- **Date**: 2026-10-03
- **Deciders**: Scrum of Scrums; needs approval from at least 3 experienced engineers from different teams (charter §7)

## Context

`docs/TECH_ARCHITECTURE.md` §1.2 calls for a multi-process architecture
(Browser, Renderer, Network, GPU, Storage, DevTools processes) "from day
one". After one month the code is a single binary with no process boundary
and no IPC transport, and every team is still building the first version of
its crate. Standing up process isolation now would cost weeks of
integration-team time and would slow every team's inner loop (two processes
to debug instead of one) before there is anything worth isolating.

At the same time, the reasons the charter wanted multiple processes
(crash isolation, security sandboxing, parallelism) are real and we do not
want to design them out.

## Decision

1. The browser is a **single process** for the foreseeable future. `browser`
   owns one tokio runtime (for `net`) and drives the engine pipeline on one
   thread.
2. Every cross-crate boundary that the charter's process model would have
   turned into IPC (`webapis ↔ storage`, renderer-side `↔ net`,
   `browser ↔ devtools`) must be **IPC-shaped**: the types that cross it are
   plain data (`Clone`, no borrows into another crate's state, no closures),
   and the call is request → response. `common::ipc::StorageRequest/Response`
   is the model.
3. Crash isolation is pursued through the type system first: no `unwrap` on
   untrusted input, no reachable `todo!()`, errors surfaced as
   `BrowserError` and shown in the chrome.
4. Revisit at the start of the next semester, or earlier if a team has a
   concrete need (for example a JIT or GPU work that wants its own process).

## Consequences

- Easier: every team can run and debug the whole browser with `cargo run`.
  Integration tests are plain Rust tests. No IPC serialization format to
  choose yet.
- Harder: nothing enforces the "IPC-shaped" rule except review. Scrum of
  Scrums checks it in the PR template's "Architecture fit" section.
- Follow-up: update charter §1.2 to say "target" rather than "from day one"
  and link this ADR. The IPC-format ADR the charter asked for is deferred
  until a process split is scheduled.

## Alternatives considered

- **Multi-process now**, as the charter says. Rejected for cost and because
  there is no sandboxing or crash-isolation benefit until pages actually
  load.
- **Threads per subsystem instead of processes.** Rejected for now; it adds
  `Send`/`Sync` constraints to every type with no isolation benefit. The
  IPC-shaped rule leaves this option open.
