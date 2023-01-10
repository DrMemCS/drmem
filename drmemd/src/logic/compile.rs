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
use tracing::error;

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

// Evaluates an expression and returns the computed value. If the
// function returns `None`, there was an error in the expression and
// it won't get computed ever again. The log will have a message
// indicating what the error was.

pub fn eval(e: &Expr) -> Option<Value> {
    match e {
        // Literals hold actual `Values`, so simply return it.
        Expr::Lit(v) => Some(v.clone()),

        // XXX: For now, we return 0.0 for device readings. When we
        // later pull in the actual readings, this will change.
        Expr::Var(_) => Some(Value::Flt(0.0)),

        // NOT expressions complement a boolean value.
        Expr::Not(ref e) => match eval(e) {
            Some(Value::Bool(v)) => Some(Value::Bool(!v)),
            Some(v) => {
                error!("NOT expression contained non-boolean value : {}", &v);
                None
            }
            None => None,
        },

        // AND expressions. If the first subexpression is `false`, the
        // second subexpression isn't evaluated.
        Expr::And(ref a, ref b) => match eval(a) {
            Some(Value::Bool(false)) => Some(Value::Bool(false)),
            Some(Value::Bool(true)) => match eval(b) {
                Some(Value::Bool(v)) => Some(Value::Bool(v)),
                Some(v) => {
                    error!(
                        "AND expression contained non-boolean argument: {}",
                        &v
                    );
                    None
                }
                None => None,
            },
            Some(v) => {
                error!("AND expression contained non-boolean argument: {}", &v);
                None
            }
            None => None,
        },

        _ => todo!(),
    }
}

// This function takes an expression and tries to reduce it.

pub fn optimize(e: Expr) -> Expr {
    match e {
        // Look for optimizations with expressions starting with NOT.
        Expr::Not(ref ne) => match **ne {
            // If the sub-expression is also a NOT expression. If so,
            // we throw them both away.
            Expr::Not(ref e) => optimize(*e.clone()),

            // If the subexpression is either `true` or `false`,
            // return the complement.
            Expr::Lit(ref v) => match v {
                Value::Bool(false) => Expr::Lit(Value::Bool(true)),
                Value::Bool(true) => Expr::Lit(Value::Bool(false)),
                _ => e,
            },
            _ => e,
        },
        _ => e,
    }
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

    // Test the evaluating function.

    #[test]
    fn test_eval() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);
	const ONE: Value = Value::Int(1);

        assert_eq!(eval(&Expr::Lit(FALSE)), Some(FALSE));
        assert_eq!(eval(&Expr::Not(Box::new(Expr::Lit(FALSE)))), Some(TRUE));
        assert_eq!(eval(&Expr::Not(Box::new(Expr::Lit(ONE)))), None);

        assert_eq!(
            eval(&Expr::And(
                Box::new(Expr::Lit(FALSE)),
                Box::new(Expr::Lit(FALSE))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::And(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(FALSE))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::And(
                Box::new(Expr::Lit(FALSE)),
                Box::new(Expr::Lit(TRUE))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::And(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(TRUE))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::And(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(TRUE))
            )),
            None
        );
	// This is a loophole for expression errors. If the first
	// subexpression is `false`, we don't evaluate the second so
	// we won't catch type errors until the first subexpression is
	// `true`.
        assert_eq!(
            eval(&Expr::And(
                Box::new(Expr::Lit(FALSE)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::And(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(ONE))
            )),
            None
        );
    }

    // This function tests the optimizations that can be done on an
    // expression.

    #[test]
    fn test_optimizer() {
        assert_eq!(
            optimize(Expr::Not(Box::new(Expr::Lit(Value::Bool(true))))),
            Expr::Lit(Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::Not(Box::new(Expr::Lit(Value::Bool(false))))),
            Expr::Lit(Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Not(Box::new(Expr::Not(Box::new(Expr::Lit(
                Value::Bool(true)
            )))))),
            Expr::Lit(Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Not(Box::new(Expr::Not(Box::new(Expr::Not(
                Box::new(Expr::Lit(Value::Bool(true)))
            )))))),
            Expr::Lit(Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::Not(Box::new(Expr::Not(Box::new(Expr::Not(
                Box::new(Expr::Not(Box::new(Expr::Lit(Value::Bool(true)))))
            )))))),
            Expr::Lit(Value::Bool(true))
        );
    }
}
