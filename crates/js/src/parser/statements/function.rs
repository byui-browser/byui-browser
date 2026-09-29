// TODO(parser): Add function-declaration token parsing once the lexer exposes
// `function`, identifiers, parameter delimiters, and braces.

use crate::ast::Statement;

impl Statement {
    pub(super) fn function_declaration(
        name: impl Into<String>,
        params: Vec<String>,
        body: Vec<Self>,
    ) -> Self {
        Self::FunctionDeclaration {
            name: name.into(),
            params,
            body,
        }
    }
}
