use super::super::{ParseResult, Parser};
use crate::ast::Expr;
use crate::lexer::Token;

/// Parses an array literal such as `[1, , x]`. Empty positions between commas
/// are holes, and one trailing comma is allowed and ignored.
pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    parser.expect(&Token::LeftBracket, "array literal")?;
    let mut elements = Vec::new();
    let mut depth = 0;
    while !parser.consume(&Token::RightBracket) {
        if parser.consume(&Token::Comma) {
            elements.push(None);
            continue;
        }
        let (element, element_depth) = parser.parse_expression_with_depth(0)?;
        elements.push(Some(element));
        depth = depth.max(element_depth);
        if !parser.consume(&Token::Comma) {
            parser.expect(&Token::RightBracket, "array literal")?;
            break;
        }
    }
    let depth = parser.node_depth(depth + 1, "array literal")?;
    Ok((Expr::Array(elements), depth))
}
