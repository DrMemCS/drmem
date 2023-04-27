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
//
//     not EXPR          Computes the complement of a boolean expression
//     EXPR or EXPR      Computes the boolean OR of two boolean expressions
//     EXPR and EXPR     Computes the boolean AND of two boolean expressions
//
//     =,<>,<,<=,>,>=    Perform the comparison and return a boolean
//
//     +,-,*,/,%         Perform addition, substraction, multiplication,
//                       division, and modulo operations

use drmem_api::{
    types::{device::Value, Error},
    Result,
};
use lrlex::lrlex_mod;
use lrpar::lrpar_mod;
use std::fmt;
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

    // NotEq, Gt, and GtEq are parsed and converted into one of the
    // following three representations (the NotEq is a combination Not
    // and Eq value.)
    Eq(Box<Expr>, Box<Expr>),
    Lt(Box<Expr>, Box<Expr>),
    LtEq(Box<Expr>, Box<Expr>),

    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),

    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Rem(Box<Expr>, Box<Expr>),
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Lit(v) => write!(f, "{}", &v),
            Expr::Var(v) => write!(f, "{{{}}}", &v),
            Expr::Not(e) => write!(f, "not ({})", &e),
            Expr::And(a, b) => write!(f, "({}) and ({})", &a, &b),
            Expr::Or(a, b) => write!(f, "({}) or ({})", &a, &b),
            Expr::Eq(a, b) => write!(f, "({}) = ({})", &a, &b),
            Expr::Lt(a, b) => write!(f, "({}) < ({})", &a, &b),
            Expr::LtEq(a, b) => write!(f, "({}) <= ({})", &a, &b),
            Expr::Add(a, b) => write!(f, "({}) + ({})", &a, &b),
            Expr::Sub(a, b) => write!(f, "({}) - ({})", &a, &b),
            Expr::Mul(a, b) => write!(f, "({}) * ({})", &a, &b),
            Expr::Div(a, b) => write!(f, "({}) / ({})", &a, &b),
            Expr::Rem(a, b) => write!(f, "({}) % ({})", &a, &b),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Program(Expr, String);

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {{{}}}", &self.0, &self.1)
    }
}

pub fn compile(s: &str) -> Result<Program> {
    let lexerdef = logic_l::lexerdef();
    let lexer = lexerdef.lexer(s);
    let (res, _) = logic_y::parse(&lexer);

    res.unwrap_or(Err(Error::BadConfig(format!(
        "expression '{}' couldn't compile",
        s
    ))))
}

// Evaluates an expression and returns the computed value. If the
// function returns `None`, there was an error in the expression and
// it won't get computed ever again. The log will have a message
// indicating what the error was.

pub fn eval(e: &Expr) -> Option<Value> {
    match e {
        // Literals hold actual `Values`, so simply return it.
        Expr::Lit(v) => Some(v.clone()),

        Expr::Var(n) => eval_as_var(n),

        Expr::Not(ref e) => eval_as_not_expr(e),

        Expr::Or(ref a, ref b) => eval_as_or_expr(a, b),

        Expr::And(ref a, ref b) => eval_as_and_expr(a, b),

        Expr::Eq(ref a, ref b) => eval_as_eq_expr(a, b),

        Expr::Lt(ref a, ref b) => eval_as_lt_expr(a, b),

        Expr::LtEq(ref a, ref b) => eval_as_lteq_expr(a, b),

        Expr::Add(ref a, ref b) => eval_as_add_expr(a, b),

        Expr::Sub(ref a, ref b) => eval_as_sub_expr(a, b),

        Expr::Mul(ref a, ref b) => eval_as_mul_expr(a, b),

        Expr::Div(ref a, ref b) => eval_as_div_expr(a, b),

        Expr::Rem(ref a, ref b) => eval_as_rem_expr(a, b),
    }
}

// XXX: Until we add the variable look-up table, we'll just return 0.0
// for variable references.

fn eval_as_var(_name: &str) -> Option<Value> {
    Some(Value::Flt(0.0))
}

// Evaluates the subexpression of a NOT expression. It only accepts
// booleans as values and simply complements the value.

fn eval_as_not_expr(e: &Expr) -> Option<Value> {
    match eval(e) {
        Some(Value::Bool(v)) => Some(Value::Bool(!v)),
        Some(v) => {
            error!("NOT expression contains non-boolean value : {}", &v);
            None
        }
        None => None,
    }
}

// OR expressions. If the first subexpression is `true`, the second
// subexpression isn't evaluated.
fn eval_as_or_expr(a: &Expr, b: &Expr) -> Option<Value> {
    match eval(a) {
        v @ Some(Value::Bool(true)) => v,
        Some(Value::Bool(false)) => match eval(b) {
            v @ Some(Value::Bool(_)) => v,
            Some(v) => {
                error!("OR expression contains non-boolean argument: {}", &v);
                None
            }
            None => None,
        },
        Some(v) => {
            error!("OR expression contains non-boolean argument: {}", &v);
            None
        }
        None => None,
    }
}

// AND expressions. If the first subexpression is `false`, the second
// subexpression isn't evaluated.
fn eval_as_and_expr(a: &Expr, b: &Expr) -> Option<Value> {
    match eval(a) {
        v @ Some(Value::Bool(false)) => v,
        Some(Value::Bool(true)) => match eval(b) {
            v @ Some(Value::Bool(_)) => v,
            Some(v) => {
                error!("AND expression contains non-boolean argument: {}", &v);
                None
            }
            None => None,
        },
        Some(v) => {
            error!("AND expression contains non-boolean argument: {}", &v);
            None
        }
        None => None,
    }
}

// EQ expressions. Both expressions must be of the same type.
fn eval_as_eq_expr(a: &Expr, b: &Expr) -> Option<Value> {
    match (eval(a), eval(b)) {
        (Some(Value::Bool(a)), Some(Value::Bool(b))) => {
            Some(Value::Bool(a == b))
        }
        (Some(Value::Int(a)), Some(Value::Int(b))) => Some(Value::Bool(a == b)),
        (Some(Value::Flt(a)), Some(Value::Flt(b))) => Some(Value::Bool(a == b)),
        (Some(Value::Int(a)), Some(Value::Flt(b))) => {
            Some(Value::Bool(a as f64 == b))
        }
        (Some(Value::Flt(a)), Some(Value::Int(b))) => {
            Some(Value::Bool(a == b as f64))
        }
        (Some(Value::Str(a)), Some(Value::Str(b))) => Some(Value::Bool(a == b)),
        (Some(a), Some(b)) => {
            error!("cannot compare {} and {} for equality", &a, &b);
            None
        }
        _ => None,
    }
}

// LT expressions. Both expressions must be of the same type.
fn eval_as_lt_expr(a: &Expr, b: &Expr) -> Option<Value> {
    match (eval(a), eval(b)) {
        (Some(Value::Int(a)), Some(Value::Int(b))) => Some(Value::Bool(a < b)),
        (Some(Value::Flt(a)), Some(Value::Flt(b))) => Some(Value::Bool(a < b)),
        (Some(Value::Int(a)), Some(Value::Flt(b))) => {
            Some(Value::Bool((a as f64) < b))
        }
        (Some(Value::Flt(a)), Some(Value::Int(b))) => {
            Some(Value::Bool(a < b as f64))
        }
        (Some(Value::Str(a)), Some(Value::Str(b))) => Some(Value::Bool(a < b)),
        (Some(a), Some(b)) => {
            error!("cannot compare {} and {} for order", &a, &b);
            None
        }
        _ => None,
    }
}

// LT_EQ expressions. Both expressions must be of the same type.
fn eval_as_lteq_expr(a: &Expr, b: &Expr) -> Option<Value> {
    match (eval(a), eval(b)) {
        (Some(Value::Int(a)), Some(Value::Int(b))) => Some(Value::Bool(a <= b)),
        (Some(Value::Flt(a)), Some(Value::Flt(b))) => Some(Value::Bool(a <= b)),
        (Some(Value::Int(a)), Some(Value::Flt(b))) => {
            Some(Value::Bool((a as f64) <= b))
        }
        (Some(Value::Flt(a)), Some(Value::Int(b))) => {
            Some(Value::Bool(a <= b as f64))
        }
        (Some(Value::Str(a)), Some(Value::Str(b))) => Some(Value::Bool(a <= b)),
        (Some(a), Some(b)) => {
            error!("cannot compare {} and {} for order", &a, &b);
            None
        }
        _ => None,
    }
}

// ADD expressions.
fn eval_as_add_expr(a: &Expr, b: &Expr) -> Option<Value> {
    match (eval(a), eval(b)) {
        (Some(Value::Int(a)), Some(Value::Int(b))) => Some(Value::Int(a + b)),
        (Some(Value::Bool(a)), Some(Value::Int(b))) => {
            Some(Value::Int(a as i32 + b))
        }
        (Some(Value::Int(a)), Some(Value::Bool(b))) => {
            Some(Value::Int(a + b as i32))
        }
        (Some(Value::Flt(a)), Some(Value::Flt(b))) => Some(Value::Flt(a + b)),
        (Some(Value::Bool(a)), Some(Value::Flt(b))) => {
            Some(Value::Flt(a as u8 as f64 + b))
        }
        (Some(Value::Flt(a)), Some(Value::Bool(b))) => {
            Some(Value::Flt(a + b as u8 as f64))
        }
        (Some(Value::Int(a)), Some(Value::Flt(b))) => {
            Some(Value::Flt((a as f64) + b))
        }
        (Some(Value::Flt(a)), Some(Value::Int(b))) => {
            Some(Value::Flt(a + b as f64))
        }
        (Some(a), Some(b)) => {
            error!("cannot add {} and {} types together", &a, &b);
            None
        }
        _ => None,
    }
}

// SUB expressions.
fn eval_as_sub_expr(a: &Expr, b: &Expr) -> Option<Value> {
    match (eval(a), eval(b)) {
        (Some(Value::Int(a)), Some(Value::Int(b))) => Some(Value::Int(a - b)),
        (Some(Value::Bool(a)), Some(Value::Int(b))) => {
            Some(Value::Int(a as i32 - b))
        }
        (Some(Value::Int(a)), Some(Value::Bool(b))) => {
            Some(Value::Int(a - b as i32))
        }
        (Some(Value::Flt(a)), Some(Value::Flt(b))) => Some(Value::Flt(a - b)),
        (Some(Value::Bool(a)), Some(Value::Flt(b))) => {
            Some(Value::Flt(a as u8 as f64 - b))
        }
        (Some(Value::Flt(a)), Some(Value::Bool(b))) => {
            Some(Value::Flt(a - b as u8 as f64))
        }
        (Some(Value::Int(a)), Some(Value::Flt(b))) => {
            Some(Value::Flt((a as f64) - b))
        }
        (Some(Value::Flt(a)), Some(Value::Int(b))) => {
            Some(Value::Flt(a - b as f64))
        }
        (Some(a), Some(b)) => {
            error!("cannot subtract {} and {} types together", &a, &b);
            None
        }
        _ => None,
    }
}

// MUL expressions.
fn eval_as_mul_expr(a: &Expr, b: &Expr) -> Option<Value> {
    match (eval(a), eval(b)) {
        (Some(Value::Int(a)), Some(Value::Int(b))) => Some(Value::Int(a * b)),
        (Some(Value::Bool(a)), Some(Value::Int(b))) => {
            Some(Value::Int(a as i32 * b))
        }
        (Some(Value::Int(a)), Some(Value::Bool(b))) => {
            Some(Value::Int(a * b as i32))
        }
        (Some(Value::Flt(a)), Some(Value::Flt(b))) => Some(Value::Flt(a * b)),
        (Some(Value::Bool(a)), Some(Value::Flt(b))) => {
            Some(Value::Flt(a as u8 as f64 * b))
        }
        (Some(Value::Flt(a)), Some(Value::Bool(b))) => {
            Some(Value::Flt(a * b as u8 as f64))
        }
        (Some(Value::Int(a)), Some(Value::Flt(b))) => {
            Some(Value::Flt((a as f64) * b))
        }
        (Some(Value::Flt(a)), Some(Value::Int(b))) => {
            Some(Value::Flt(a * b as f64))
        }
        (Some(a), Some(b)) => {
            error!("cannot multiply {} and {} types together", &a, &b);
            None
        }
        _ => None,
    }
}

// DIV expressions.
fn eval_as_div_expr(a: &Expr, b: &Expr) -> Option<Value> {
    match (eval(a), eval(b)) {
        (Some(Value::Int(a)), Some(Value::Int(b))) if b != 0 => {
            Some(Value::Int(a / b))
        }
        (Some(Value::Flt(a)), Some(Value::Flt(b))) if b != 0.0 => {
            Some(Value::Flt(a / b))
        }
        (Some(Value::Int(a)), Some(Value::Flt(b))) if b != 0.0 => {
            Some(Value::Flt((a as f64) / b))
        }
        (Some(Value::Flt(a)), Some(Value::Int(b))) if b != 0 => {
            Some(Value::Flt(a / b as f64))
        }
        (Some(a), Some(b)) => {
            error!("cannot divide {} by {}", &a, &b);
            None
        }
        _ => None,
    }
}

// REM expressions.
fn eval_as_rem_expr(a: &Expr, b: &Expr) -> Option<Value> {
    match (eval(a), eval(b)) {
        (Some(Value::Int(a)), Some(Value::Int(b))) if b > 0 => {
            Some(Value::Int(a % b))
        }
        (Some(Value::Flt(a)), Some(Value::Flt(b))) if b > 0.0 => {
            Some(Value::Flt(a % b))
        }
        (Some(Value::Int(a)), Some(Value::Flt(b))) if b > 0.0 => {
            Some(Value::Flt((a as f64) % b))
        }
        (Some(Value::Flt(a)), Some(Value::Int(b))) if b > 0 => {
            Some(Value::Flt(a % b as f64))
        }
        (Some(a), Some(b)) => {
            error!("cannot compute remainder of {} from {}", &b, &a);
            None
        }
        _ => None,
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

        Expr::And(ref a, ref b) => {
            match (optimize(*a.clone()), optimize(*b.clone())) {
                (v @ Expr::Lit(Value::Bool(false)), _)
                | (_, v @ Expr::Lit(Value::Bool(false))) => v,
                (
                    v @ Expr::Lit(Value::Bool(true)),
                    Expr::Lit(Value::Bool(true)),
                ) => v,
                (Expr::Lit(Value::Bool(true)), e)
                | (e, Expr::Lit(Value::Bool(true))) => e,
                _ => e,
            }
        }

        Expr::Or(ref a, ref b) => {
            match (optimize(*a.clone()), optimize(*b.clone())) {
                (v @ Expr::Lit(Value::Bool(true)), _)
                | (_, v @ Expr::Lit(Value::Bool(true))) => v,
                (
                    v @ Expr::Lit(Value::Bool(false)),
                    Expr::Lit(Value::Bool(false)),
                ) => v,
                (Expr::Lit(Value::Bool(false)), e)
                | (e, Expr::Lit(Value::Bool(false))) => e,
                _ => e,
            }
        }

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
        assert!(compile("switch} -> {bulb}").is_err());

        assert_eq!(
            compile("{switch} -> {bulb}"),
            Ok(Program(
                Expr::Var(String::from("switch")),
                String::from("bulb")
            ))
        );

        assert_eq!(
            compile("true -> {bulb}"),
            Ok(Program(Expr::Lit(Value::Bool(true)), String::from("bulb")))
        );
        assert_eq!(
            compile("false -> {bulb}"),
            Ok(Program(Expr::Lit(Value::Bool(false)), String::from("bulb")))
        );

        assert_eq!(
            compile("1 -> {bulb}"),
            Ok(Program(Expr::Lit(Value::Int(1)), String::from("bulb")))
        );
        assert_eq!(
            compile("1. -> {bulb}"),
            Ok(Program(Expr::Lit(Value::Flt(1.0)), String::from("bulb")))
        );
        assert_eq!(
            compile("1.0 -> {bulb}"),
            Ok(Program(Expr::Lit(Value::Flt(1.0)), String::from("bulb")))
        );
        assert_eq!(
            compile("-1.0 -> {bulb}"),
            Ok(Program(Expr::Lit(Value::Flt(-1.0)), String::from("bulb")))
        );
        assert_eq!(
            compile("1.5 -> {bulb}"),
            Ok(Program(Expr::Lit(Value::Flt(1.5)), String::from("bulb")))
        );
        assert_eq!(
            compile("1.0e10 -> {bulb}"),
            Ok(Program(Expr::Lit(Value::Flt(1.0e10)), String::from("bulb")))
        );
        assert_eq!(
            compile("2.75e-10 -> {bulb}"),
            Ok(Program(
                Expr::Lit(Value::Flt(2.75e-10)),
                String::from("bulb")
            ))
        );
        assert_eq!(
            compile("(((10))) -> {bulb}"),
            Ok(Program(Expr::Lit(Value::Int(10)), String::from("bulb")))
        );

        assert_eq!(
            compile("{on_time} > 10.0 -> {bulb}"),
            Ok(Program(
                Expr::Lt(
                    Box::new(Expr::Lit(Value::Flt(10.0))),
                    Box::new(Expr::Var(String::from("on_time")))
                ),
                String::from("bulb")
            ))
        );

        assert_eq!(
            compile("4 + ({on_time} + 5) * 10 > 10.0 % 3 -> {bulb}"),
            Ok(Program(
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
            Ok(Program(
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

        assert_eq!(
            compile("\"Hello, world!\" -> {bulb}"),
            Ok(Program(
                Expr::Lit(Value::Str("Hello, world!".to_string())),
                String::from("bulb")
            ))
        );
    }

    #[test]
    fn test_eval_not_expr() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);

        assert_eq!(eval(&Expr::Not(Box::new(Expr::Lit(FALSE)))), Some(TRUE));
        assert_eq!(eval(&Expr::Not(Box::new(Expr::Lit(TRUE)))), Some(FALSE));
        assert_eq!(eval(&Expr::Not(Box::new(Expr::Lit(Value::Int(1))))), None);
    }

    #[test]
    fn test_eval_or_expr() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);
        const ONE: Value = Value::Int(1);

        assert_eq!(
            eval(&Expr::Or(
                Box::new(Expr::Lit(FALSE)),
                Box::new(Expr::Lit(FALSE))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::Or(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(FALSE))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::Or(
                Box::new(Expr::Lit(FALSE)),
                Box::new(Expr::Lit(TRUE))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::Or(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(TRUE))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::Or(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(TRUE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Or(
                Box::new(Expr::Lit(FALSE)),
                Box::new(Expr::Lit(ONE))
            )),
            None
        );
        // This is a loophole for expression errors. If the first
        // subexpression is `true`, we don't evaluate the second so
        // we won't catch type errors until the first subexpression is
        // `false`.
        assert_eq!(
            eval(&Expr::Or(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(TRUE)
        );
    }

    #[test]
    fn test_eval_and_expr() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);
        const ONE: Value = Value::Int(1);

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

    #[test]
    fn test_eval_eq_expr() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);
        const ONE: Value = Value::Int(1);
        const TWO: Value = Value::Int(2);
        const FP_ONE: Value = Value::Flt(1.0);

        assert_eq!(
            eval(&Expr::Eq(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::Eq(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(TWO))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::Eq(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::Eq(
                Box::new(Expr::Lit(FP_ONE)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::Eq(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Eq(
                Box::new(Expr::Lit(Value::Str(String::from("same")))),
                Box::new(Expr::Lit(Value::Str(String::from("same"))))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::Eq(
                Box::new(Expr::Lit(Value::Str(String::from("same")))),
                Box::new(Expr::Lit(Value::Str(String::from("not same"))))
            )),
            Some(FALSE)
        );
    }

    #[test]
    fn test_eval_lt_expr() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);
        const ONE: Value = Value::Int(1);
        const TWO: Value = Value::Int(2);
        const FP_ONE: Value = Value::Flt(1.0);

        assert_eq!(
            eval(&Expr::Lt(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::Lt(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(TWO))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::Lt(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(TWO))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::Lt(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Lt(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::Lt(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::Lt(
                Box::new(Expr::Lit(FP_ONE)),
                Box::new(Expr::Lit(TWO))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::Lt(
                Box::new(Expr::Lit(Value::Str(String::from("abc")))),
                Box::new(Expr::Lit(Value::Str(String::from("abc"))))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::Lt(
                Box::new(Expr::Lit(Value::Str(String::from("abc")))),
                Box::new(Expr::Lit(Value::Str(String::from("abcd"))))
            )),
            Some(TRUE)
        );
    }

    #[test]
    fn test_eval_lteq_expr() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);
        const ONE: Value = Value::Int(1);
        const TWO: Value = Value::Int(2);
        const FP_ONE: Value = Value::Flt(1.0);

        assert_eq!(
            eval(&Expr::LtEq(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::LtEq(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(TWO))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::LtEq(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(TWO))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::LtEq(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::LtEq(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::LtEq(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::LtEq(
                Box::new(Expr::Lit(FP_ONE)),
                Box::new(Expr::Lit(TWO))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::LtEq(
                Box::new(Expr::Lit(Value::Str(String::from("abcd")))),
                Box::new(Expr::Lit(Value::Str(String::from("abc"))))
            )),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::LtEq(
                Box::new(Expr::Lit(Value::Str(String::from("abc")))),
                Box::new(Expr::Lit(Value::Str(String::from("abc"))))
            )),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::LtEq(
                Box::new(Expr::Lit(Value::Str(String::from("abc")))),
                Box::new(Expr::Lit(Value::Str(String::from("abcd"))))
            )),
            Some(TRUE)
        );
    }

    #[test]
    fn test_eval_add_expr() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);
        const ONE: Value = Value::Int(1);
        const TWO: Value = Value::Int(2);
        const FP_ONE: Value = Value::Flt(1.0);
        const FP_TWO: Value = Value::Flt(2.0);

        assert_eq!(
            eval(&Expr::Add(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(Value::Int(3))
        );
        assert_eq!(
            eval(&Expr::Add(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(Value::Int(2))
        );
        assert_eq!(
            eval(&Expr::Add(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            Some(Value::Int(1))
        );
        assert_eq!(
            eval(&Expr::Add(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(Value::Flt(3.0))
        );
        assert_eq!(
            eval(&Expr::Add(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(Value::Flt(2.0))
        );
        assert_eq!(
            eval(&Expr::Add(
                Box::new(Expr::Lit(FP_ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            Some(Value::Flt(1.0))
        );
        assert_eq!(
            eval(&Expr::Add(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(Value::Flt(3.0))
        );
        assert_eq!(
            eval(&Expr::Add(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(Value::Flt(3.0))
        );
    }

    #[test]
    fn test_eval_sub_expr() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);
        const ONE: Value = Value::Int(1);
        const TWO: Value = Value::Int(2);
        const FP_ONE: Value = Value::Flt(1.0);
        const FP_TWO: Value = Value::Flt(2.0);

        assert_eq!(
            eval(&Expr::Sub(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(Value::Int(1))
        );
        assert_eq!(
            eval(&Expr::Sub(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(Value::Int(0))
        );
        assert_eq!(
            eval(&Expr::Sub(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            Some(Value::Int(1))
        );
        assert_eq!(
            eval(&Expr::Sub(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(Value::Flt(1.0))
        );
        assert_eq!(
            eval(&Expr::Sub(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(Value::Flt(0.0))
        );
        assert_eq!(
            eval(&Expr::Sub(
                Box::new(Expr::Lit(FP_ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            Some(Value::Flt(1.0))
        );
        assert_eq!(
            eval(&Expr::Sub(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(Value::Flt(1.0))
        );
        assert_eq!(
            eval(&Expr::Sub(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(Value::Flt(1.0))
        );
    }

    #[test]
    fn test_eval_mul_expr() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);
        const ONE: Value = Value::Int(1);
        const TWO: Value = Value::Int(2);
        const FP_ONE: Value = Value::Flt(1.0);
        const FP_TWO: Value = Value::Flt(2.0);

        assert_eq!(
            eval(&Expr::Mul(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(Value::Int(2))
        );
        assert_eq!(
            eval(&Expr::Mul(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(Value::Int(1))
        );
        assert_eq!(
            eval(&Expr::Mul(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            Some(Value::Int(0))
        );
        assert_eq!(
            eval(&Expr::Mul(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(Value::Flt(2.0))
        );
        assert_eq!(
            eval(&Expr::Mul(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(Value::Flt(1.0))
        );
        assert_eq!(
            eval(&Expr::Mul(
                Box::new(Expr::Lit(FP_ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            Some(Value::Flt(0.0))
        );
        assert_eq!(
            eval(&Expr::Mul(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(Value::Flt(2.0))
        );
        assert_eq!(
            eval(&Expr::Mul(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(Value::Flt(2.0))
        );
    }

    #[test]
    fn test_eval_div_expr() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);
        const ZERO: Value = Value::Int(0);
        const ONE: Value = Value::Int(1);
        const TWO: Value = Value::Int(2);
        const FP_ZERO: Value = Value::Flt(0.0);
        const FP_ONE: Value = Value::Flt(1.0);
        const FP_TWO: Value = Value::Flt(2.0);

        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(Value::Int(2))
        );
        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(ONE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(Value::Flt(2.0))
        );
        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(FP_ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(ONE))
            )),
            Some(Value::Flt(2.0))
        );
        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            Some(Value::Flt(2.0))
        );
        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(ZERO))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(FP_ZERO))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(ZERO))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Div(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(FP_ZERO))
            )),
            None
        );
    }

    #[test]
    fn test_eval_rem_expr() {
        const TRUE: Value = Value::Bool(true);
        const FALSE: Value = Value::Bool(false);
        const ZERO: Value = Value::Int(0);
        const NEG_ONE: Value = Value::Int(-1);
        const ONE: Value = Value::Int(1);
        const TWO: Value = Value::Int(2);
        const FP_ZERO: Value = Value::Flt(0.0);
        const FP_ONE: Value = Value::Flt(1.0);
        const FP_TWO: Value = Value::Flt(2.0);

        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(TWO))
            )),
            Some(Value::Int(1))
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(ONE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(FP_ONE)),
                Box::new(Expr::Lit(FP_TWO))
            )),
            Some(Value::Flt(1.0))
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(TRUE)),
                Box::new(Expr::Lit(FP_ONE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(FP_ONE)),
                Box::new(Expr::Lit(FALSE))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(FP_ONE)),
                Box::new(Expr::Lit(TWO))
            )),
            Some(Value::Flt(1.0))
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(ONE)),
                Box::new(Expr::Lit(FP_TWO))
            )),
            Some(Value::Flt(1.0))
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(ZERO))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(FP_ZERO))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(ZERO))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(FP_TWO)),
                Box::new(Expr::Lit(FP_ZERO))
            )),
            None
        );
        assert_eq!(
            eval(&Expr::Rem(
                Box::new(Expr::Lit(TWO)),
                Box::new(Expr::Lit(NEG_ONE))
            )),
            None
        );
    }

    #[test]
    fn test_eval() {
        const FALSE: Value = Value::Bool(false);

        assert_eq!(eval(&Expr::Lit(FALSE)), Some(FALSE));
    }

    // This function tests the optimizations that can be done on an
    // expression.

    #[test]
    fn test_not_optimizer() {
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

    #[test]
    fn test_and_optimizer() {
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(Value::Bool(false))),
                Box::new(Expr::Lit(Value::Bool(false)))
            )),
            Expr::Lit(Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(Value::Bool(false))),
                Box::new(Expr::Lit(Value::Bool(true)))
            )),
            Expr::Lit(Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(Value::Bool(false))),
                Box::new(Expr::Lit(Value::Str(String::from("test"))))
            )),
            Expr::Lit(Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(Value::Bool(true))),
                Box::new(Expr::Lit(Value::Bool(false)))
            )),
            Expr::Lit(Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(Value::Bool(true))),
                Box::new(Expr::Lit(Value::Bool(true)))
            )),
            Expr::Lit(Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(Value::Bool(true))),
                Box::new(Expr::Lit(Value::Str(String::from("test"))))
            )),
            Expr::Lit(Value::Str(String::from("test")))
        );

        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(Value::Bool(true))),
                Box::new(Expr::And(
                    Box::new(Expr::Lit(Value::Bool(true))),
                    Box::new(Expr::Lit(Value::Bool(true)))
                ))
            )),
            Expr::Lit(Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::And(
                    Box::new(Expr::Lit(Value::Bool(true))),
                    Box::new(Expr::Lit(Value::Bool(true)))
                )),
                Box::new(Expr::Lit(Value::Bool(true)))
            )),
            Expr::Lit(Value::Bool(true))
        );

        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(Value::Bool(true))),
                Box::new(Expr::And(
                    Box::new(Expr::Lit(Value::Bool(true))),
                    Box::new(Expr::Lit(Value::Bool(false)))
                ))
            )),
            Expr::Lit(Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::And(
                    Box::new(Expr::Lit(Value::Bool(true))),
                    Box::new(Expr::Lit(Value::Bool(true)))
                )),
                Box::new(Expr::Lit(Value::Bool(false)))
            )),
            Expr::Lit(Value::Bool(false))
        );
    }

    #[test]
    fn test_or_optimizer() {
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(Value::Bool(false))),
                Box::new(Expr::Lit(Value::Bool(false)))
            )),
            Expr::Lit(Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(Value::Bool(false))),
                Box::new(Expr::Lit(Value::Bool(true)))
            )),
            Expr::Lit(Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(Value::Bool(false))),
                Box::new(Expr::Lit(Value::Str(String::from("test"))))
            )),
            Expr::Lit(Value::Str(String::from("test")))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(Value::Bool(true))),
                Box::new(Expr::Lit(Value::Bool(false)))
            )),
            Expr::Lit(Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(Value::Bool(true))),
                Box::new(Expr::Lit(Value::Bool(true)))
            )),
            Expr::Lit(Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(Value::Bool(true))),
                Box::new(Expr::Lit(Value::Str(String::from("test"))))
            )),
            Expr::Lit(Value::Bool(true))
        );

        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(Value::Bool(true))),
                Box::new(Expr::Or(
                    Box::new(Expr::Lit(Value::Bool(true))),
                    Box::new(Expr::Lit(Value::Bool(true)))
                ))
            )),
            Expr::Lit(Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Or(
                    Box::new(Expr::Lit(Value::Bool(true))),
                    Box::new(Expr::Lit(Value::Bool(true)))
                )),
                Box::new(Expr::Lit(Value::Bool(true)))
            )),
            Expr::Lit(Value::Bool(true))
        );

        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(Value::Bool(true))),
                Box::new(Expr::Or(
                    Box::new(Expr::Lit(Value::Bool(true))),
                    Box::new(Expr::Lit(Value::Bool(false)))
                ))
            )),
            Expr::Lit(Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Or(
                    Box::new(Expr::Lit(Value::Bool(true))),
                    Box::new(Expr::Lit(Value::Bool(true)))
                )),
                Box::new(Expr::Lit(Value::Bool(false)))
            )),
            Expr::Lit(Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Or(
                    Box::new(Expr::Lit(Value::Bool(false))),
                    Box::new(Expr::Lit(Value::Bool(false)))
                )),
                Box::new(Expr::Lit(Value::Bool(false)))
            )),
            Expr::Lit(Value::Bool(false))
        );
    }

    #[test]
    fn test_to_string() {
        assert_eq!(compile("{a} -> {b}").unwrap().to_string(), "{a} -> {b}");

        assert_eq!(compile("true -> {b}").unwrap().to_string(), "true -> {b}");
        assert_eq!(
            compile("not true -> {b}").unwrap().to_string(),
            "not (true) -> {b}"
        );
        assert_eq!(
            compile("{a} and {b} -> {c}").unwrap().to_string(),
            "({a}) and ({b}) -> {c}"
        );
        assert_eq!(
            compile("{a} or {b} -> {c}").unwrap().to_string(),
            "({a}) or ({b}) -> {c}"
        );
        assert_eq!(
            compile("{a} = {b} -> {c}").unwrap().to_string(),
            "({a}) = ({b}) -> {c}"
        );
        assert_eq!(
            compile("{a} < {b} -> {c}").unwrap().to_string(),
            "({a}) < ({b}) -> {c}"
        );
        assert_eq!(
            compile("{a} <= {b} -> {c}").unwrap().to_string(),
            "({a}) <= ({b}) -> {c}"
        );
        assert_eq!(
            compile("{a} + {b} -> {c}").unwrap().to_string(),
            "({a}) + ({b}) -> {c}"
        );
        assert_eq!(
            compile("{a} - {b} -> {c}").unwrap().to_string(),
            "({a}) - ({b}) -> {c}"
        );
        assert_eq!(
            compile("{a} * {b} -> {c}").unwrap().to_string(),
            "({a}) * ({b}) -> {c}"
        );
        assert_eq!(
            compile("{a} / {b} -> {c}").unwrap().to_string(),
            "({a}) / ({b}) -> {c}"
        );
        assert_eq!(
            compile("{a} % {b} -> {c}").unwrap().to_string(),
            "({a}) % ({b}) -> {c}"
        );

        assert_eq!(
            compile("{a} * 3 + {b} > 4 -> {c}").unwrap().to_string(),
            "(4) < ((({a}) * (3)) + ({b})) -> {c}"
        );
    }
}
