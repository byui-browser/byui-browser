# Rust conventions for this workspace

These apply to every crate. They exist so that code from nine teams reads as
one codebase and so that an agent can tell a deliberate placeholder from a bug.
`AGENTS.md` is the short version; this is the reasoning and the detail.

## Crate layout

- Every crate has `#![forbid(unsafe_code)]` in `lib.rs`. Removing it needs an
  ADR and two experienced reviewers (charter §1.1).
- Every crate has `#![warn(missing_docs)]` in `lib.rs`. CI runs with
  `RUSTFLAGS=-D warnings`, so a missing doc comment fails the build. Crates
  that do not have it yet add it when they next touch `lib.rs`.
- Modules are private (`mod foo;`) and the public surface is re-exported from
  `lib.rs` with `pub use`. The crate root is the contract; the module tree is
  the team's business.
- `lib.rs` opens with a `//!` doc stating the crate's purpose, owning team,
  and a link to its document under `docs/architecture/crates/`.
- One crate, one responsibility. A crate that needs a second binary or a
  second responsibility is a conversation with Scrum of Scrums, not a new
  `[[bin]]`. Demo binaries (`src/main.rs` in a library crate) are removed
  before merge; use an `examples/` directory or a test instead.

## Public API

- Every `pub` item (type, field, function, method, variant, const) has a doc
  comment stating purpose, units where relevant (CSS px, device px, ms,
  bytes), ownership (who creates it, who may mutate it), and how callers use
  it.
- Signatures that appear in a `docs/architecture/contracts/*.md` file are
  shared contracts. Changing one needs the other side's review and an update
  to that file in the same PR.
- Prefer newtypes over raw `u32`/`usize`/`String` for anything with identity
  or units: `NodeId`, `TabId`, `Url`, `Origin`. Derive `Debug, Clone, Copy,
  PartialEq, Eq, Hash` on id newtypes.
- Public structs that will grow use private fields plus accessors, or
  `#[non_exhaustive]`, so adding a field is not a breaking change.
- Do not leak third-party types in a public signature unless that crate is a
  deliberate, ADR-recorded part of the contract. (`net` leaking `reqwest`
  types is a known gap.)
- Public functions take borrowed inputs (`&Document`, `&[Stylesheet]`) and
  return owned outputs. Avoid `String` parameters where `&str` works.

## Errors

- Crate-internal errors are the crate's choice. At a boundary another team
  calls, return `common::error::BrowserError` or a crate error type that
  implements `std::error::Error` and `From`-converts into `BrowserError`.
- Never `unwrap()`, `expect()`, or index without a bounds check on data that
  came from a page, the network, or disk. Those are inputs we do not control.
  `expect()` is fine on invariants the code itself guarantees, with a message
  that names the invariant.
- Parsers for untrusted input (`html`, `css`) do not return errors. They
  recover and continue. `js` returns a `ParseError` because that is the
  language's defined behaviour.

## Placeholders and partial features

The scaffold phase left deliberate placeholders. Keep them recognisable:

- `// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team` marks a
  type or function shape that Scrum of Scrums wrote so the crates would
  compile. The owning team may reshape it freely and **removes the label**
  when they take ownership. A label left on real code is a doc bug.
- `todo!("TODO(<crate>): what is missing")` is allowed only when **all** of
  the following hold:
  1. it is unreachable from a loaded page (no `browser`/`webapis` path can
     trigger it);
  2. the public function's doc comment says the input domain it rejects;
  3. the matching contract test in `tests/contract.rs` is `#[ignore = "TODO(<crate>): ..."]`
     with the same reason.
  Otherwise return `BrowserError::Unimplemented("<crate>: what")`.
- When you implement the feature, un-ignore the contract test in the same PR.
  A contract test that would pass but is still ignored is a gap (several exist
  today; see the gap report).
- A deliberately partial feature documents its limit on the API it affects
  **and** ships a regression test for the supported subset (`AGENTS.md`).

## Tests

Three layers, each with a home:

| Layer | Where | What it proves |
| --- | --- | --- |
| Unit | `#[cfg(test)] mod tests` beside the code | one function, private details allowed |
| Contract | `crates/<crate>/tests/contract.rs` | the public API as another team sees it; only `pub` items; breaks when a signature changes |
| Integration | `tests/` at the repository root (once a `browser` library exists); until then `crates/<crate>/tests/` | a vertical slice across crates (HTML → pixels, script → console) |

Rules:

- Tests never touch the real network. `net` tests bind `127.0.0.1:0`. A test
  that needs internet is a bug.
- Tests do not write outside `target/`. A test that writes a PNG for humans
  to look at puts it under `CARGO_TARGET_DIR` and does not fail if it cannot.
- Fixtures live in `tests/html/`, `tests/css/`, `tests/js/` and are loaded
  with `include_str!`. Do not duplicate fixtures inside crates.
- `#[ignore]` always carries a reason string starting with `TODO(<crate>):`.
- `make test` and `make lint` must pass on every PR (`.github/workflows/ci.yml`).
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` should also pass;
  the `open-pr` skill runs it.

## Dependencies

The workspace currently has two third-party runtime dependencies (`reqwest`
with `rustls-tls` and `http2`, and `tokio`) plus `winit`/`softbuffer` in the
macOS shell and `png` as a dev-dependency. Keep it that way unless a
dependency clearly earns its place.

- Each new dependency is justified in the PR body (the template has a
  section): what it provides, which alternative was rejected, license,
  maintenance state, `unsafe` and build-script footprint.
- Engine crates (`html`, `css`, `layout`, `paint`, `js`) are expected to stay
  dependency-free. The course goal is to build these, not to wrap them.
- A dependency that appears in a public signature is a contract decision;
  see **Public API** above.
- Declare dependencies in `[workspace.dependencies]` and reference them with
  `workspace = true`, as the workspace already does for internal crates.
- Remove a declared dependency that the crate does not use. Several crates
  declare `common` without using it today.

## Formatting and lints

- `cargo fmt --all` with the repository `rustfmt.toml` (edition 2024,
  `max_width = 100`).
- `cargo clippy --workspace --all-targets -- -D warnings` clean. Do not add
  `#[allow(...)]` to silence a lint you do not understand; ask. A crate-wide
  `#![allow(dead_code)]` is never acceptable in a merged crate.
- No `println!`, `eprintln!`, or `dbg!` in library code. Output goes through
  the console abstraction (see `contracts/scripting.md` §Console) or a
  logging facade once one is chosen.

## Naming

- Crate names are the short lowercase names in `Cargo.toml` (`html`, `css`,
  `net`). Prefix tests and TODOs with them: `TODO(css):`.
- The DOM document type is `Document`. The computed-style map is
  `ComputedStyles`. The box tree is `LayoutTree`. The paint output is
  `DisplayList`. The rasterised output is `Frame`. Do not introduce synonyms
  (`HTMLDocument`, `Dom`, `StyledDom`) for the same concept; aliases left
  over from the scaffold are listed as gaps.
- Rust naming rules apply to everything, including methods that mirror Web
  API names: `query_selector`, not `querySelector`. The JavaScript-visible
  name is a string registered in `webapis`, not a Rust identifier.

## Git and PRs

- Branch names: `<team>/<short-description>` (for example `css/cascade`),
  with `scrum-of-scrums/` for integration work.
- Small PRs. The `open-pr` skill suggests splitting above roughly 400
  changed lines outside tests. Architecture-wide documentation PRs are the
  exception and say so in their summary.
- Fill in every section of `.github/pull_request_template.md`. "None" is an
  answer; a deleted section is not.
- AI-assisted PRs end with the disclosure line from `AGENTS.md`.
