use super::super::{ParseResult, Parser};
use crate::ast::{Expr, ObjectProperty, PropertyKey};
use crate::lexer::Token;
use crate::runtime::number_to_string;

/// Parses an object literal such as `{ a: 1, 'b': 2, 3: x, [key]: y, a }`.
/// A trailing comma is allowed, as in JavaScript.
pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    parser.expect(&Token::LeftBrace, "object literal")?;
    let mut properties = Vec::new();
    let mut depth = 0;
    while !parser.consume(&Token::RightBrace) {
        let (property, property_depth) = property(parser)?;
        properties.push(property);
        depth = depth.max(property_depth);
        if !parser.consume(&Token::Comma) {
            parser.expect(&Token::RightBrace, "object literal")?;
            break;
        }
    }
    let depth = parser.node_depth(depth + 1, "object literal")?;
    Ok((Expr::Object(properties), depth))
}

/// Parses one `key: value` entry, or a shorthand `name` entry.
fn property(parser: &mut Parser<'_>) -> ParseResult<(ObjectProperty, usize)> {
    if let Some(Token::Identifier(name)) = parser.peek() {
        let name = name.clone();
        parser.advance();
        if matches!(parser.peek(), Some(Token::Comma | Token::RightBrace)) {
            let property = ObjectProperty {
                key: PropertyKey::Named(name.clone()),
                value: Expr::Identifier(name),
            };
            return Ok((property, 1));
        }
        return value(parser, PropertyKey::Named(name), 0);
    }

    let (key, key_depth) = match parser.peek() {
        Some(Token::String(text)) => {
            let key = PropertyKey::Named(text.clone());
            parser.advance();
            (key, 0)
        }
        Some(Token::Number(number)) => {
            let key = PropertyKey::Named(number_to_string(*number));
            parser.advance();
            (key, 0)
        }
        Some(Token::LeftBracket) => {
            parser.advance();
            let (key, key_depth) = parser.parse_expression_with_depth(0)?;
            parser.expect(&Token::RightBracket, "computed property key")?;
            (PropertyKey::Computed(Box::new(key)), key_depth)
        }
        _ => (
            PropertyKey::Named(super::call::property_name(parser, "object literal")?),
            0,
        ),
    };
    value(parser, key, key_depth)
}

/// Parses the `: value` part of an entry whose key has been read.
fn value(
    parser: &mut Parser<'_>,
    key: PropertyKey,
    key_depth: usize,
) -> ParseResult<(ObjectProperty, usize)> {
    parser.expect(&Token::Colon, "object literal")?;
    let (value, value_depth) = parser.parse_expression_with_depth(0)?;
    Ok((ObjectProperty { key, value }, key_depth.max(value_depth)))
}
