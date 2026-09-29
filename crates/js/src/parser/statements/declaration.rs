// TODO(parser): Add `var`, `let`, and `const` token parsing once the lexer
// exposes declaration keywords, identifiers, and `=`.

use crate::ast::{Expr, Statement, VarKind};

impl Statement {
    pub(super) fn variable_declaration(
        kind: VarKind,
        name: impl Into<String>,
        init: Option<Expr>,
    ) -> Self {
        Self::VariableDeclaration {
            kind,
            name: name.into(),
            init,
        }
    }
}
