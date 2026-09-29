// TODO(parser): Add block-token parsing once the lexer exposes `{` and `}`.

use crate::ast::Statement;

impl Statement {
    pub(super) fn block(body: Vec<Self>) -> Self {
        Self::Block(body)
    }
}
