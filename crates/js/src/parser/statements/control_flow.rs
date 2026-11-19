use super::super::{ParseResult, Parser};
use crate::ast::Statement;
use crate::lexer::Token;

pub(super) fn parse(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    if parser.consume(&Token::For) {
        return parse_for(parser);
    }
    if parser.consume(&Token::Do) {
        parser.loops.push(parser.function_depth);
        let body = body_statement(parser)?;
        parser.loops.pop();
        parser.expect(&Token::While, "do/while statement")?;
        parser.expect(&Token::LeftParen, "do/while condition")?;
        let condition = parser.parse_sequence()?;
        parser.expect(&Token::RightParen, "do/while condition")?;
        parser.terminator("do/while statement")?;
        return Ok(Statement::DoWhile {
            body: Box::new(body),
            condition,
        });
    }
    if parser.consume(&Token::Switch) {
        return parse_switch(parser);
    }
    let is_if = parser.consume(&Token::If);
    if !is_if {
        parser.expect(&Token::While, "while statement")?;
    }
    parser.expect(&Token::LeftParen, "condition")?;
    let condition = parser.parse_sequence()?;
    parser.expect(&Token::RightParen, "condition")?;
    if !is_if {
        parser.loops.push(parser.function_depth);
    }
    let body = body_statement(parser);
    if !is_if {
        parser.loops.pop();
    }
    let body = Box::new(body?);
    if is_if {
        let else_branch = if parser.consume(&Token::Else) {
            Some(Box::new(body_statement(parser)?))
        } else {
            None
        };
        Ok(Statement::If {
            condition,
            then_branch: body,
            else_branch,
        })
    } else {
        Ok(Statement::While { condition, body })
    }
}

fn parse_for(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    parser.expect(&Token::LeftParen, "for statement")?;
    let init = if parser.consume(&Token::Semicolon) {
        None
    } else if matches!(parser.peek(), Some(Token::Var | Token::Let | Token::Const)) {
        let kind = match parser.advance() {
            Some(Token::Var) => crate::ast::VarKind::Var,
            Some(Token::Let) => crate::ast::VarKind::Let,
            Some(Token::Const) => crate::ast::VarKind::Const,
            _ => unreachable!(),
        };
        let name = parser.identifier("for initializer")?;
        let init = if parser.consume(&Token::Assign) {
            Some(parser.parse_expression(0)?)
        } else {
            None
        };
        let is_of = parser.consume(&Token::Of);
        let is_in = !is_of && parser.consume(&Token::In);
        if is_of || is_in {
            let iterable = parser.parse_sequence()?;
            parser.expect(&Token::RightParen, "for iteration")?;
            parser.loops.push(parser.function_depth);
            let body = body_statement(parser);
            parser.loops.pop();
            return Ok(if is_of {
                Statement::ForOf {
                    kind,
                    name,
                    iterable,
                    body: Box::new(body?),
                }
            } else {
                Statement::ForIn {
                    kind,
                    name,
                    object: iterable,
                    body: Box::new(body?),
                }
            });
        }
        if kind == crate::ast::VarKind::Const && init.is_none() {
            return Err(parser.error("Constant requires an initializer", "for initializer"));
        }
        parser.expect(&Token::Semicolon, "for initializer")?;
        Some(Box::new(Statement::VariableDeclaration {
            kind,
            name,
            init,
        }))
    } else {
        let expression = parser.parse_sequence()?;
        parser.expect(&Token::Semicolon, "for initializer")?;
        Some(Box::new(Statement::Expression(expression)))
    };
    let condition = if parser.consume(&Token::Semicolon) {
        None
    } else {
        let expression = parser.parse_sequence()?;
        parser.expect(&Token::Semicolon, "for condition")?;
        Some(expression)
    };
    let update = if parser.consume(&Token::RightParen) {
        None
    } else {
        let expression = parser.parse_sequence()?;
        parser.expect(&Token::RightParen, "for update")?;
        Some(expression)
    };
    parser.loops.push(parser.function_depth);
    let body = body_statement(parser);
    parser.loops.pop();
    Ok(Statement::For {
        init,
        condition,
        update,
        body: Box::new(body?),
    })
}

fn parse_switch(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    parser.expect(&Token::LeftParen, "switch expression")?;
    let discriminant = parser.parse_sequence()?;
    parser.expect(&Token::RightParen, "switch expression")?;
    parser.expect(&Token::LeftBrace, "switch statement")?;
    parser.breakables.push(parser.function_depth);
    let mut cases = Vec::new();
    let mut seen_default = false;
    while !parser.consume(&Token::RightBrace) {
        let test = if parser.consume(&Token::Case) {
            Some(parser.parse_sequence()?)
        } else if parser.consume(&Token::Default) {
            if seen_default {
                return Err(parser.error("Duplicate default clause", "switch statement"));
            }
            seen_default = true;
            None
        } else {
            return Err(parser.error("Expected case or default", "switch statement"));
        };
        parser.expect(&Token::Colon, "switch clause")?;
        let mut consequent = Vec::new();
        while !matches!(
            parser.peek(),
            Some(Token::Case | Token::Default | Token::RightBrace) | None
        ) {
            consequent.push(parser.parse_statement()?);
        }
        cases.push(crate::ast::SwitchCase { test, consequent });
    }
    parser.breakables.pop();
    Ok(Statement::Switch {
        discriminant,
        cases,
    })
}

/// Parses a single-statement body, which may not be a lexical or function
/// declaration because it has no block scope to hold the binding.
fn body_statement(parser: &mut Parser<'_>) -> ParseResult<Statement> {
    if matches!(
        parser.peek(),
        Some(Token::Let | Token::Const | Token::Function)
    ) {
        return Err(parser.error(
            "Declaration cannot appear in a single-statement context",
            "statement body",
        ));
    }
    parser.parse_statement()
}
