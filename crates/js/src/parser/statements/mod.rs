//! Statement parsing.
//!
//! Variable declarations, function declarations, and other statement forms
//! belong in this module. Their implementations can be added once the
//! corresponding tokens and AST nodes are available.

// TODO(parser): Add statement dispatch once the lexer exposes statement
// keywords, delimiters, and terminators.

#[allow(dead_code)]
mod block;
#[allow(dead_code)]
mod control_flow;
#[allow(dead_code)]
mod declaration;
#[allow(dead_code)]
mod function;
#[allow(dead_code)]
mod return_statement;
