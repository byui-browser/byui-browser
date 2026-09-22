//! Public-contract tests for `layout`.
//!
//! Run ignored tests with `cargo test -p layout -- --ignored` to see the backlog.

use common::ids::NodeId;
use layout::{Size, StyledDom, layout_tree};

#[test]
#[ignore = "TODO(layout): block layout not implemented"]
fn block_children_stack_vertically() {
    let dom = StyledDom {
        nodes: vec![NodeId::new(0), NodeId::new(1)],
    };
    let tree = layout_tree(
        &dom,
        Size {
            width: 100.0,
            height: 100.0,
        },
    );
    let [first, second] = tree.root.children.as_slice() else {
        panic!("expected two block children");
    };
    assert_eq!(first.rect.y, 0.0);
    assert!(second.rect.y >= first.rect.y + first.rect.height);
}
