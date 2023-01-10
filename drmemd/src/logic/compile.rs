// This module contains the lexer and parser for the logic node
// language. It's main responsibility is to take expressions and
// convert them into a form that can be run in a background task.
//
// The language supports the following primitive types:
//
//     ##                integers
//     #.##              floating point numbers
//     {NAME}            variable named NAME (from config params)
//
// The token "->" represents assignment. The only items that can be on
// the right hand side of the arrow is a variable referring to a
// settable device.
//
// Parentheses can be used to group subexpressions.

use drmem_api::types::device::Value;
use lrlex::lrlex_mod;
use lrpar::lrpar_mod;

// Pull in the lexer and parser for the Logic Node language.

lrlex_mod!("logic/logic.l");
lrpar_mod!("logic/logic.y");

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Lit(Value),
    Var(String),

    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),

    Eq(Box<Expr>, Box<Expr>),
    Lt(Box<Expr>, Box<Expr>),
    LtEq(Box<Expr>, Box<Expr>),

    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),

    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Rem(Box<Expr>, Box<Expr>),
}

#[derive(Debug, PartialEq)]
pub enum Program {
    Assign(Expr, String),
}

pub fn compile(s: &str) -> Result<Program, ()> {
    let lexerdef = logic_l::lexerdef();
    let lexer = lexerdef.lexer(s);
    let (res, _) = logic_y::parse(&lexer);

    res.unwrap_or_else(|| Err(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser() {
        assert!(compile("").is_err());
        assert!(compile("{switch -> {bulb}").is_err());

        assert_eq!(
            compile("{switch} -> {bulb}"),
            Ok(Program::Assign(
                Expr::Var(String::from("switch")),
                String::from("bulb")
            ))
        );

        assert_eq!(
            compile("true -> {bulb}"),
            Ok(Program::Assign(
                Expr::Lit(Value::Bool(true)),
                String::from("bulb")
            ))
        );
        assert_eq!(
            compile("false -> {bulb}"),
            Ok(Program::Assign(
                Expr::Lit(Value::Bool(false)),
                String::from("bulb")
            ))
        );

        assert_eq!(
            compile("1 -> {bulb}"),
            Ok(Program::Assign(
                Expr::Lit(Value::Int(1)),
                String::from("bulb")
            ))
        );
        assert_eq!(
            compile("1. -> {bulb}"),
            Ok(Program::Assign(
                Expr::Lit(Value::Flt(1.0)),
                String::from("bulb")
            ))
        );
        assert_eq!(
            compile("1.0 -> {bulb}"),
            Ok(Program::Assign(
                Expr::Lit(Value::Flt(1.0)),
                String::from("bulb")
            ))
        );
        assert_eq!(
            compile("(((10))) -> {bulb}"),
            Ok(Program::Assign(
                Expr::Lit(Value::Int(10)),
                String::from("bulb")
            ))
        );

        assert_eq!(
            compile("{on_time} > 10.0 -> {bulb}"),
            Ok(Program::Assign(
                Expr::Lt(
                    Box::new(Expr::Lit(Value::Flt(10.0))),
                    Box::new(Expr::Var(String::from("on_time")))
                ),
                String::from("bulb")
            ))
        );

        assert_eq!(
            compile("4 + ({on_time} + 5) * 10 > 10.0 % 3 -> {bulb}"),
            Ok(Program::Assign(
                Expr::Lt(
                    Box::new(Expr::Rem(
                        Box::new(Expr::Lit(Value::Flt(10.0))),
                        Box::new(Expr::Lit(Value::Int(3)))
                    )),
                    Box::new(Expr::Add(
                        Box::new(Expr::Lit(Value::Int(4))),
                        Box::new(Expr::Mul(
                            Box::new(Expr::Add(
                                Box::new(Expr::Var(String::from("on_time"))),
                                Box::new(Expr::Lit(Value::Int(5)))
                            )),
                            Box::new(Expr::Lit(Value::Int(10)))
                        ))
                    ))
                ),
                String::from("bulb")
            ))
        );

        assert_eq!(
            compile("true and false or false and true -> {bulb}"),
            Ok(Program::Assign(
                Expr::Or(
                    Box::new(Expr::And(
                        Box::new(Expr::Lit(Value::Bool(true))),
                        Box::new(Expr::Lit(Value::Bool(false)))
                    )),
                    Box::new(Expr::And(
                        Box::new(Expr::Lit(Value::Bool(false))),
                        Box::new(Expr::Lit(Value::Bool(true)))
                    ))
                ),
                String::from("bulb")
            ))
        );
    }
}
