

# Browser Tech Architecture

**Project**: Independent web browser built from scratch  
**Primary Language**: Rust (edition 2024+)  
**Date**: July 2026  
**Team Size**: ~37 engineers (10 experienced + 27 juniors) across 9 specialized teams

This document is the project **charter**: principles, team ownership, review rules, workflow, and testing strategy. It is the source of truth for how teams collaborate.

The **technical** detail (layering and dependency rules, the per-crate contracts, what each crate currently is versus what it is meant to be) lives in [`docs/architecture/`](architecture/README.md); start at its README, which routes you to the documents for your task. Where the two disagree, `docs/architecture/` is more recent and the disagreement is an open ADR: see [`architecture/overview.md`](architecture/overview.md) §7 and [`adr/`](adr/README.md). Two proposed ADRs (0001 single-process-first, 0002 DOM representation) would revise §1.2 and §3.1 below once accepted.

---

## 1. High-Level Architecture

### 1.1 Core Principles

- **Memory safety first**: Prefer safe Rust. `unsafe` requires justification + review by at least two experienced engineers.
- **Clear ownership boundaries**: Every piece of code has a single owning team.
- **Message-passing over shared mutable state**: Especially across process boundaries.
- **Incremental correctness**: Prefer correct + slow over fast + wrong. Performance is optimized after correctness and tests exist.
- **Platform-agnostic core**: The engine produces pixels + events. Platform shells are thin.

### 1.2 Process Model

Modern multi-process architecture from day one:


| Process            | Responsibility                                 | Owning Team(s)                                        |
| ------------------ | ---------------------------------------------- | ----------------------------------------------------- |
| Browser (UI)       | Chrome, tabs, navigation, UX                   | Browser UX                                            |
| Renderer (per tab) | HTML/CSS/JS/Layout/Paint for one document      | HTML, CSS, Layout &amp; Rendering, JS Engine, JS APIs |
| Network            | All HTTP(S), caching, cookies                  | Networking                                            |
| GPU / Compositor   | Layer composition, GPU resource management     | Layout &amp; Rendering                                |
| Utility / Storage  | IndexedDB, localStorage, service workers, etc. | Security &amp; Storage                                |
| DevTools           | Inspector, debugger, profiler                  | Devtools                                              |


IPC uses a typed, versioned protocol (recommended: `ipc-channel` + serde or a custom binary protocol with schema evolution).

### 1.3 Monorepo Structure (Cargo Workspace)

```
browser/
├── Cargo.toml                 # Workspace root
├── crates/
│   ├── common/                # Shared types, error handling, logging, IPC definitions
│   ├── html/                  # HTML parser + DOM construction
│   ├── css/                   # CSS parser, cascade, computed style
│   ├── layout/                # Box tree, layout algorithms
│   ├── paint/                 # Display list generation
│   ├── render/                # GPU rendering (wgpu), compositor
│   ├── js/                    # JavaScript engine (parser, bytecode, VM, later JIT)
│   ├── webapis/               # DOM bindings, Window, Document, Fetch, etc.
│   ├── net/                   # Networking stack
│   ├── storage/               # Storage APIs + security policy
│   ├── security/              # Sandboxing, process isolation helpers, CSP, etc.
│   ├── chrome/                # Browser UI (or platform shells)
│   ├── devtools/              # DevTools protocol + UI
│   └── browser/               # Top-level binary that wires everything
├── platforms/                 # Optional thin native shells (macOS AppKit, Windows, GTK)
└── tests/                     # Integration + web-platform-tests subset
```

Each major crate is owned by one primary team. The `common` crate is jointly maintained with strict change control.

---

## 2. Team Ownership &amp; Responsibilities


| Team                            | Primary Crates                        | Key Responsibilities                                                                     |
| ------------------------------- | ------------------------------------- | ---------------------------------------------------------------------------------------- |
| **HTML Team**                   | `html`, parts of `common` (DOM nodes) | HTML tokenizer, tree builder, DOM construction, mutation observers, basic DOM APIs       |
| **CSS Engine Team**             | `css`                                 | CSS parser, stylesheet management, cascade, computed values, style invalidation          |
| **Layout &amp; Rendering Team** | `layout`, `paint`, `render`           | Box tree, layout (block/inline/flex/grid), painting, compositor, GPU backend             |
| **JavaScript Engine Team**      | `js`                                  | Parser, AST, bytecode compiler, interpreter/VM, garbage collector, later JIT             |
| **JS APIs (Web APIs) Team**     | `webapis`                             | Bindings between JS and DOM/platform (Window, Document, Element, Fetch, etc.)            |
| **Networking Team**             | `net`                                 | HTTP/1.1 + HTTP/2 + HTTP/3, TLS (rustls), caching, connection pooling, cookies           |
| **Security &amp; Storage Team** | `security`, `storage`                 | Process sandboxing, site isolation, CSP, permissions, IndexedDB, localStorage, Cache API |
| **Browser UX Team**             | `chrome`, platform shells             | Tabs, address bar, menus, settings, bookmarks, window management, accessibility          |
| **Devtools Team**               | `devtools`                            | Protocol, inspector frontend, debugger, profiler, network panel, console                 |


**Rule**: A team may only merge changes into crates they own without cross-team review. Changes that touch another team's crate require explicit approval from that team.

---

## 3. Inter-Team Contracts &amp; Interaction Patterns

### 3.1 Shared Types (`common` crate)

- All cross-crate types live here: `NodeId`, `Style`, `LayoutBox`, `DisplayList`, `IPC messages`, error types, etc.
- Changes to public types in `common` require review from **all** affected teams + at least one experienced engineer.
- Prefer newtype wrappers and strong typing over raw integers/strings.

### 3.2 Key Interfaces Between Teams

**HTML → CSS**

- HTML team produces a DOM tree (arena-allocated).
- CSS team walks the DOM + stylesheets and produces computed style maps.
- Interface: `css::style_document(dom: &Dom, stylesheets: &[Stylesheet]) -> ComputedStyles`

**CSS → Layout**

- Layout receives the styled DOM and builds the box tree + performs layout.
- Interface: `layout::layout_tree(styled_dom: &StyledDom, viewport: Size) -> LayoutTree`

**Layout → Paint / Render**

- Layout produces a display list or layer tree.
- Paint/Render turns it into GPU commands or a framebuffer.
- Clear separation: Layout never talks directly to the GPU.

**JS Engine ↔ Web APIs ↔ DOM**

- JS Engine owns the VM and values.
- Web APIs team owns the bindings (how JS objects map to Rust DOM nodes).
- HTML/CSS/Layout teams expose safe, minimal APIs that Web APIs can call.
- All DOM mutations from JS go through a controlled mutation interface so style/layout invalidation stays correct.

**Networking ↔ Everyone**

- Networking exposes a high-level async API: `fetch(request) -> Response`.
- Security &amp; Storage team owns cookie jar and cache policy decisions.
- Renderer processes never open raw sockets; all network goes through the Network process.

**Security boundaries**

- Renderer processes are heavily sandboxed.
- Any new capability (file access, device APIs, etc.) must be reviewed by Security &amp; Storage.

### 3.3 Communication Mechanisms

1. **In-process (same renderer)**: Direct function calls + shared ownership via `Arc` / arenas where appropriate.
2. **Cross-process**: Typed IPC messages only. No shared memory for complex structures initially.
3. **Events / invalidation**: Explicit invalidation messages (style dirty, layout dirty) rather than observers that can create cycles.

---

## 4. Development Workflow

### 4.1 Branching &amp; Pull Requests

- Trunk-based development on `main`.
- Feature branches named `team/short-description` (e.g. `css/cascade-layers`).
- Small, focused PRs preferred.
- Cross-team PRs must have reviewers from every owning team whose code is touched.
- Experienced engineers act as code owners for critical paths (parser, layout core, JS VM, IPC, security).

### 4.2 Required Checks Before Merge

- `cargo fmt` + `cargo clippy -- -D warnings`
- Unit tests for the changed crate
- Integration tests that exercise the new path (when applicable)
- For parser/style/layout/JS changes: relevant web-platform-tests or internal conformance tests
- Miri for any new `unsafe` or complex ownership

### 4.3 Coordination Cadence

- **Weekly Architecture Sync** (30–45 min): Representatives from all teams. Decisions recorded as Architecture Decision Records (ADRs) in `/docs/adr/`.
- **Bi-weekly Cross-team Demo**: Show progress on integration points.
- **Async first**: Prefer written proposals (GitHub Discussions or RFCs in-repo) for significant interface changes.

### 4.4 Handling Breaking Changes

1. Propose the change in an RFC or Discussion.
2. Get buy-in from affected teams.
3. Land the new API alongside the old one (if possible).
4. Migrate call sites.
5. Remove the old API in a follow-up PR.

---

## 5. Testing Strategy


| Level             | Owner                        | Tools / Approach                                                  |
| ----------------- | ---------------------------- | ----------------------------------------------------------------- |
| Unit tests        | Each team                    | `cargo test` inside crate                                         |
| Integration tests | Cross-team                   | `tests/` directory, full pipeline (HTML → pixels or JS execution) |
| Conformance       | Relevant teams               | Subset of WPT, test262 (JS), CSS test suite                       |
| Fuzzing           | HTML, CSS, JS, Net, Security | `cargo-fuzz` on parsers and deserializers                         |
| Performance       | Layout &amp; Rendering + JS  | Criterion benchmarks, real-page traces                            |
| Security          | Security &amp; Storage       | Sandbox escape tests, process isolation tests                     |


Renderer processes should be crash-isolated: a panic in one tab must not take down the browser.

---

## 6. Onboarding &amp; Junior Engineer Guidelines

- Every junior is paired with an experienced mentor on their team for the first 3 months.
- Start with well-scoped tasks that have clear interfaces (e.g. implementing a specific CSS property, a DOM method, a network cache rule).
- Prefer adding tests and documentation over large refactors early on.
- `unsafe` is off-limits for juniors without explicit mentor approval.
- Use the type system aggressively — if the compiler is happy and the tests pass, the change is usually safe to land.

---

## 7. Evolution Rules

This document is living. Changes require:

1. A short written proposal.
2. Discussion in the weekly Architecture Sync.
3. Approval from at least 3 experienced engineers from different teams.
4. Update to this file + corresponding ADR.

**Priority order when trade-offs arise**:

1. Security &amp; correctness
2. Clear team boundaries and maintainability
3. Performance
4. Feature completeness

---

## 8. Immediate Next Steps

1. Create the Cargo workspace and empty crates with ownership comments.
2. Define the core shared types in `common` (NodeId, basic DOM skeleton, IPC message enum).
3. Stand up a minimal multi-process skeleton (Browser process + one Renderer process that can load a static HTML string and paint a solid color).
4. Agree on the first vertical slice: parse simple HTML → apply basic CSS → layout a block → paint to a window.
5. Write the first set of ADRs for process model, IPC format, and DOM representation.

---

*This architecture prioritizes long-term velocity and safety over short-term feature speed. With 27 junior engineers, clear boundaries and strong type contracts are the highest-leverage investment we can make.*