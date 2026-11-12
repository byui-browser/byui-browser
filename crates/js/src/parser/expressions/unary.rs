use super::super::{ParseResult, Parser};
use crate::ast::{Expr, UnaryOperator, UpdateOperator};
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<(Expr, usize)> {
    let update = match parser.peek() {
        Some(Token::Increment) => Some(UpdateOperator::Increment),
        Some(Token::Decrement) => Some(UpdateOperator::Decrement),
        _ => None,
    };
    if let Some(operator) = update {
        parser.advance();
        return parser.nested("update expression", |parser| {
            let (target, depth) = parse(parser)?;
            if !matches!(target, Expr::Identifier(_) | Expr::Member { .. }) {
                return Err(parser.error(
                    "Invalid left-hand side expression in prefix operation",
                    "update expression",
                ));
            }
            let depth = parser.node_depth(depth + 1, "update expression")?;
            Ok((
                Expr::Update {
                    target: Box::new(target),
                    operator,
                    prefix: true,
                },
                depth,
            ))
        });
    }

    if parser.peek() == Some(&Token::Delete) {
        parser.advance();
        return parser.nested("delete expression", |parser| {
            let (operand, depth) = parse(parser)?;
            match operand {
                Expr::Identifier(_) => {
                    return Err(parser.error(
                        "Delete of an unqualified identifier is not supported",
                        "delete expression",
                    ));
                }
                Expr::OptionalChain(_) => {
                    return Err(parser.error(
                        "Delete of an optional chain is not supported",
                        "delete expression",
                    ));
                }
                _ => {}
            }
            reject_exponent_after_unary(parser)?;
            let depth = parser.node_depth(depth + 1, "delete expression")?;
            Ok((Expr::Delete(Box::new(operand)), depth))
        });
    }

    let operator = match parser.peek() {
        Some(Token::Subtract) => Some(UnaryOperator::Negate),
        Some(Token::Plus) => Some(UnaryOperator::Plus),
        Some(Token::Bang) => Some(UnaryOperator::Not),
        Some(Token::BitNot) => Some(UnaryOperator::BitwiseNot),
        Some(Token::Typeof) => Some(UnaryOperator::Typeof),
        Some(Token::Void) => Some(UnaryOperator::Void),
        _ => None,
    };
    if let Some(operator) = operator {
        parser.advance();
        return parser.nested("unary expression", |parser| {
            let (operand, depth) = parse(parser)?;
            reject_exponent_after_unary(parser)?;
            let depth = parser.node_depth(depth + 1, "unary expression")?;
            Ok((
                Expr::Unary {
                    operator,
                    operand: Box::new(operand),
                },
                depth,
            ))
        });
    }

    let (expression, depth) = super::call::parse(parser)?;
    let postfix = match parser.peek() {
        Some(Token::Increment) => Some(UpdateOperator::Increment),
        Some(Token::Decrement) => Some(UpdateOperator::Decrement),
        _ => None,
    };
    // A line terminator before `++`/`--` ends the statement instead, so
    // `a\n++b` is `a; ++b`.
    match postfix {
        Some(operator) if !parser.line_terminator_before_current() => {
            if !matches!(expression, Expr::Identifier(_) | Expr::Member { .. }) {
                return Err(parser.error(
                    "Invalid left-hand side expression in postfix operation",
                    "update expression",
                ));
            }
            parser.advance();
            let depth = parser.node_depth(depth + 1, "update expression")?;
            Ok((
                Expr::Update {
                    target: Box::new(expression),
                    operator,
                    prefix: false,
                },
                depth,
            ))
        }
        _ => Ok((expression, depth)),
    }
}

/// `-a ** b` is ambiguous, so JavaScript requires `(-a) ** b` or `-(a ** b)`.
fn reject_exponent_after_unary(parser: &Parser<'_>) -> ParseResult<()> {
    if parser.peek() == Some(&Token::StarStar) {
        return Err(parser.error(
            "A unary operator before `**` needs parentheses",
            "exponentiation",
        ));
    }
    Ok(())
}
