//! Public-contract tests for `common`.
//!
//! These only see the public API. If a signature changes, this file is what
//! breaks, which is the signal we want for jointly owned types.

use common::error::BrowserError;


#[test]
fn browser_error_is_a_std_error() {
    let err: Box<dyn std::error::Error> = Box::new(BrowserError::Unimplemented("ipc"));
    assert!(err.to_string().contains("ipc"));
}
