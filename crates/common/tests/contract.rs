//! Public-contract tests for `common`.
//!
//! These only see the public API. If a signature changes, this file is what
//! breaks, which is the signal we want for jointly owned types.

use std::collections::HashMap;

use common::error::BrowserError;
use common::ids::NodeId;

#[test]
fn node_id_works_as_map_key() {
    let mut map = HashMap::new();
    map.insert(NodeId::new(1), "html");
    map.insert(NodeId::new(2), "body");
    assert_eq!(map.get(&NodeId::new(2)), Some(&"body"));
}

#[test]
fn browser_error_is_a_std_error() {
    let err: Box<dyn std::error::Error> = Box::new(BrowserError::Unimplemented("ipc"));
    assert!(err.to_string().contains("ipc"));
}
