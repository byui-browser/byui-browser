//! Public-contract tests for `devtools`.
//!
//! Run ignored tests with `cargo test -p devtools -- --ignored` to see the backlog.

use devtools::{Console, Level};

#[test]
#[ignore = "TODO(devtools): level filtering not implemented"]
fn filters_entries_by_level() {
    let mut console = Console::new();
    console.log(Level::Log, "a");
    console.log(Level::Warn, "b");
    console.log(Level::Warn, "c");
    let warnings = console.entries_at(Level::Warn);
    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0].message, "b");
    assert_eq!(warnings[1].message, "c");
}
