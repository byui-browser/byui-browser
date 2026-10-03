# Architecture Decision Records (ADRs)

Significant architecture decisions from the Weekly Architecture Sync are recorded here.

## Process

1. Propose the change (Discussion, RFC, or draft ADR).
2. Discuss in the Weekly Architecture Sync.
3. Get approval from at least 3 experienced engineers from different teams.
4. Land the ADR and update `docs/TECH_ARCHITECTURE.md` and the affected
   `docs/architecture/` documents when needed.

## Naming

`NNNN-short-title.md` — zero-padded number, kebab-case title.

Example: `0001-process-model.md`

## Template

Copy [`0000-template.md`](./0000-template.md) when starting a new ADR.

## Index

| ADR | Title | Status |
| --- | --- | --- |
| [0001](0001-single-process-first.md) | Single-process browser with IPC-shaped boundaries | Proposed |
| [0002](0002-dom-representation.md) | DOM representation | Proposed |

Decisions still needed are listed in `docs/architecture/overview.md` §7
(shared-type ownership, script asynchrony, UI toolkit, GPU backend, cookie
jar ownership). The IPC-format ADR from `docs/TECH_ARCHITECTURE.md` §8 is
deferred by ADR 0001.
