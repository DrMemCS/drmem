// This module contains the lexer and parser for the logic node
// language. It's main responsibility is to take expressions and
// convert them into a form that can be run in a background task.
//
// The language supports the following primitive types:
//
//     true/false        booleans
//     ##                integers
//     #.##              floating point numbers
//     "TEXT"            strings
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

use drmem_api::{device, Error, Result};
use lrlex::lrlex_mod;
use lrpar::lrpar_mod;
use std::fmt;
use tracing::error;

// Pull in the lexer and parser for the Logic Node language.

lrlex_mod!("logic/logic.l");
lrpar_mod!("logic/logic.y");

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Lit(device::Value),
    Var(usize),

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
            Expr::Var(v) => write!(f, "inp[{}]", &v),
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

// This is the "environment" of a compile. The first element is a list
// of names associated with devices that are to be read. The second
// element is a list of names associated with devices to be set. The
// `Program::compile` function uses this environment to compute the
// `Expr::Var` and `Program` values.

type Env<'a> = (&'a [String], &'a [String]);

#[derive(Debug, PartialEq)]
pub struct Program(pub Expr, pub usize);

impl Program {
    pub fn optimize(self) -> Self {
        Program(optimize(self.0), self.1)
    }

    pub fn compile(s: &str, env: &Env) -> Result<Program> {
        let lexerdef = logic_l::lexerdef();
        let lexer = lexerdef.lexer(s);
        let (res, errs) = logic_y::parse(&lexer, env);

        res.unwrap_or_else(|| {
            let res = errs.iter().fold(
                format!("expression '{}' couldn't compile", s),
                |mut acc, e| {
                    acc.push_str(&format!("\n    {}", &e));
                    acc
                },
            );

            Err(Error::ParseError(res))
        })
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> out[{}]", &self.0, &self.1)
    }
}

// Evaluates an expression and returns the computed value. If the
// function returns `None`, there was an error in the expression and
// it won't get computed ever again. The log will have a message
// indicating what the error was.

pub fn eval(e: &Expr, inp: &[Option<device::Value>]) -> Option<device::Value> {
    match e {
        // Literals hold actual `device::Values`, so simply return it.
        Expr::Lit(v) => Some(v.clone()),

        Expr::Var(n) => eval_as_var(*n, inp),

        Expr::Not(ref e) => eval_as_not_expr(e, inp),

        Expr::Or(ref a, ref b) => eval_as_or_expr(a, b, inp),

        Expr::And(ref a, ref b) => eval_as_and_expr(a, b, inp),

        Expr::Eq(ref a, ref b) => eval_as_eq_expr(a, b, inp),

        Expr::Lt(ref a, ref b) => eval_as_lt_expr(a, b, inp),

        Expr::LtEq(ref a, ref b) => eval_as_lteq_expr(a, b, inp),

        Expr::Add(ref a, ref b) => eval_as_add_expr(a, b, inp),

        Expr::Sub(ref a, ref b) => eval_as_sub_expr(a, b, inp),

        Expr::Mul(ref a, ref b) => eval_as_mul_expr(a, b, inp),

        Expr::Div(ref a, ref b) => eval_as_div_expr(a, b, inp),

        Expr::Rem(ref a, ref b) => eval_as_rem_expr(a, b, inp),
    }
}

// Returns the latest value of the variable.

fn eval_as_var(
    idx: usize, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    inp[idx].clone()
}

// Evaluates the subexpression of a NOT expression. It only accepts
// booleans as values and simply complements the value.

fn eval_as_not_expr(
    e: &Expr, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    match eval(e, inp) {
        Some(device::Value::Bool(v)) => Some(device::Value::Bool(!v)),
        Some(v) => {
            error!("NOT expression contains non-boolean value : {}", &v);
            None
        }
        None => None,
    }
}

// OR expressions. If the first subexpression is `true`, the second
// subexpression isn't evaluated.
fn eval_as_or_expr(
    a: &Expr, b: &Expr, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    match eval(a, inp) {
        v @ Some(device::Value::Bool(true)) => v,
        Some(device::Value::Bool(false)) => match eval(b, inp) {
            v @ Some(device::Value::Bool(_)) => v,
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
fn eval_as_and_expr(
    a: &Expr, b: &Expr, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    match eval(a, inp) {
        v @ Some(device::Value::Bool(false)) => v,
        Some(device::Value::Bool(true)) => match eval(b, inp) {
            v @ Some(device::Value::Bool(_)) => v,
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
fn eval_as_eq_expr(
    a: &Expr, b: &Expr, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    match (eval(a, inp), eval(b, inp)) {
        (Some(device::Value::Bool(a)), Some(device::Value::Bool(b))) => {
            Some(device::Value::Bool(a == b))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Bool(a == b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Bool(a == b))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Bool(a as f64 == b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Bool(a == b as f64))
        }
        (Some(device::Value::Str(a)), Some(device::Value::Str(b))) => {
            Some(device::Value::Bool(a == b))
        }
        (Some(a), Some(b)) => {
            error!("cannot compare {} and {} for equality", &a, &b);
            None
        }
        _ => None,
    }
}

// LT expressions. Both expressions must be of the same type.
fn eval_as_lt_expr(
    a: &Expr, b: &Expr, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    match (eval(a, inp), eval(b, inp)) {
        (Some(device::Value::Int(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Bool(a < b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Bool(a < b))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Bool((a as f64) < b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Bool(a < b as f64))
        }
        (Some(device::Value::Str(a)), Some(device::Value::Str(b))) => {
            Some(device::Value::Bool(a < b))
        }
        (Some(a), Some(b)) => {
            error!("cannot compare {} and {} for order", &a, &b);
            None
        }
        _ => None,
    }
}

// LT_EQ expressions. Both expressions must be of the same type.
fn eval_as_lteq_expr(
    a: &Expr, b: &Expr, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    match (eval(a, inp), eval(b, inp)) {
        (Some(device::Value::Int(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Bool(a <= b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Bool(a <= b))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Bool((a as f64) <= b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Bool(a <= b as f64))
        }
        (Some(device::Value::Str(a)), Some(device::Value::Str(b))) => {
            Some(device::Value::Bool(a <= b))
        }
        (Some(a), Some(b)) => {
            error!("cannot compare {} and {} for order", &a, &b);
            None
        }
        _ => None,
    }
}

// ADD expressions.
fn eval_as_add_expr(
    a: &Expr, b: &Expr, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    match (eval(a, inp), eval(b, inp)) {
        (Some(device::Value::Int(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Int(a + b))
        }
        (Some(device::Value::Bool(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Int(a as i32 + b))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Bool(b))) => {
            Some(device::Value::Int(a + b as i32))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Flt(a + b))
        }
        (Some(device::Value::Bool(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Flt(a as u8 as f64 + b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Bool(b))) => {
            Some(device::Value::Flt(a + b as u8 as f64))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Flt((a as f64) + b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Flt(a + b as f64))
        }
        (Some(a), Some(b)) => {
            error!("cannot add {} and {} types together", &a, &b);
            None
        }
        _ => None,
    }
}

// SUB expressions.
fn eval_as_sub_expr(
    a: &Expr, b: &Expr, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    match (eval(a, inp), eval(b, inp)) {
        (Some(device::Value::Int(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Int(a - b))
        }
        (Some(device::Value::Bool(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Int(a as i32 - b))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Bool(b))) => {
            Some(device::Value::Int(a - b as i32))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Flt(a - b))
        }
        (Some(device::Value::Bool(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Flt(a as u8 as f64 - b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Bool(b))) => {
            Some(device::Value::Flt(a - b as u8 as f64))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Flt((a as f64) - b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Flt(a - b as f64))
        }
        (Some(a), Some(b)) => {
            error!("cannot subtract {} and {} types together", &a, &b);
            None
        }
        _ => None,
    }
}

// MUL expressions.
fn eval_as_mul_expr(
    a: &Expr, b: &Expr, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    match (eval(a, inp), eval(b, inp)) {
        (Some(device::Value::Int(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Int(a * b))
        }
        (Some(device::Value::Bool(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Int(a as i32 * b))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Bool(b))) => {
            Some(device::Value::Int(a * b as i32))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Flt(a * b))
        }
        (Some(device::Value::Bool(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Flt(a as u8 as f64 * b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Bool(b))) => {
            Some(device::Value::Flt(a * b as u8 as f64))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Flt(b))) => {
            Some(device::Value::Flt((a as f64) * b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Int(b))) => {
            Some(device::Value::Flt(a * b as f64))
        }
        (Some(a), Some(b)) => {
            error!("cannot multiply {} and {} types together", &a, &b);
            None
        }
        _ => None,
    }
}

// DIV expressions.
fn eval_as_div_expr(
    a: &Expr, b: &Expr, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    match (eval(a, inp), eval(b, inp)) {
        (Some(device::Value::Int(a)), Some(device::Value::Int(b)))
            if b != 0 =>
        {
            Some(device::Value::Int(a / b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Flt(b)))
            if b != 0.0 =>
        {
            Some(device::Value::Flt(a / b))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Flt(b)))
            if b != 0.0 =>
        {
            Some(device::Value::Flt((a as f64) / b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Int(b)))
            if b != 0 =>
        {
            Some(device::Value::Flt(a / b as f64))
        }
        (Some(a), Some(b)) => {
            error!("cannot divide {} by {}", &a, &b);
            None
        }
        _ => None,
    }
}

// REM expressions.
fn eval_as_rem_expr(
    a: &Expr, b: &Expr, inp: &[Option<device::Value>],
) -> Option<device::Value> {
    match (eval(a, inp), eval(b, inp)) {
        (Some(device::Value::Int(a)), Some(device::Value::Int(b))) if b > 0 => {
            Some(device::Value::Int(a % b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Flt(b)))
            if b > 0.0 =>
        {
            Some(device::Value::Flt(a % b))
        }
        (Some(device::Value::Int(a)), Some(device::Value::Flt(b)))
            if b > 0.0 =>
        {
            Some(device::Value::Flt((a as f64) % b))
        }
        (Some(device::Value::Flt(a)), Some(device::Value::Int(b))) if b > 0 => {
            Some(device::Value::Flt(a % b as f64))
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
                device::Value::Bool(false) => {
                    Expr::Lit(device::Value::Bool(true))
                }
                device::Value::Bool(true) => {
                    Expr::Lit(device::Value::Bool(false))
                }
                _ => e,
            },
            _ => e,
        },

        Expr::And(ref a, ref b) => {
            match (optimize(*a.clone()), optimize(*b.clone())) {
                (v @ Expr::Lit(device::Value::Bool(false)), _)
                | (_, v @ Expr::Lit(device::Value::Bool(false))) => v,
                (
                    v @ Expr::Lit(device::Value::Bool(true)),
                    Expr::Lit(device::Value::Bool(true)),
                ) => v,
                (Expr::Lit(device::Value::Bool(true)), e)
                | (e, Expr::Lit(device::Value::Bool(true))) => e,
                _ => e,
            }
        }

        Expr::Or(ref a, ref b) => {
            match (optimize(*a.clone()), optimize(*b.clone())) {
                (v @ Expr::Lit(device::Value::Bool(true)), _)
                | (_, v @ Expr::Lit(device::Value::Bool(true))) => v,
                (
                    v @ Expr::Lit(device::Value::Bool(false)),
                    Expr::Lit(device::Value::Bool(false)),
                ) => v,
                (Expr::Lit(device::Value::Bool(false)), e)
                | (e, Expr::Lit(device::Value::Bool(false))) => e,
                _ => e,
            }
        }

        _ => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drmem_api::device;

    #[test]
    fn test_parser() {
        let env: Env = (
            &[String::from("switch"), String::from("on_time")],
            &[String::from("bulb")],
        );

        assert!(Program::compile("", &env).is_err());
        assert!(Program::compile("{switch -> {bulb}", &env).is_err());
        assert!(Program::compile("switch} -> {bulb}", &env).is_err());

        assert_eq!(
            Program::compile("{switch} -> {bulb}", &env),
            Ok(Program(Expr::Var(0), 0))
        );

        assert_eq!(
            Program::compile("true -> {bulb}", &env),
            Ok(Program(Expr::Lit(device::Value::Bool(true)), 0))
        );
        assert_eq!(
            Program::compile("false -> {bulb}", &env),
            Ok(Program(Expr::Lit(device::Value::Bool(false)), 0))
        );

        assert_eq!(
            Program::compile("1 -> {bulb}", &env),
            Ok(Program(Expr::Lit(device::Value::Int(1)), 0))
        );
        assert_eq!(
            Program::compile("1. -> {bulb}", &env),
            Ok(Program(Expr::Lit(device::Value::Flt(1.0)), 0))
        );
        assert_eq!(
            Program::compile("1.0 -> {bulb}", &env),
            Ok(Program(Expr::Lit(device::Value::Flt(1.0)), 0))
        );
        assert_eq!(
            Program::compile("-1.0 -> {bulb}", &env),
            Ok(Program(Expr::Lit(device::Value::Flt(-1.0)), 0))
        );
        assert_eq!(
            Program::compile("1.5 -> {bulb}", &env),
            Ok(Program(Expr::Lit(device::Value::Flt(1.5)), 0))
        );
        assert_eq!(
            Program::compile("1.0e10 -> {bulb}", &env),
            Ok(Program(Expr::Lit(device::Value::Flt(1.0e10)), 0))
        );
        assert_eq!(
            Program::compile("2.75e-10 -> {bulb}", &env),
            Ok(Program(Expr::Lit(device::Value::Flt(2.75e-10)), 0))
        );
        assert_eq!(
            Program::compile("(((10))) -> {bulb}", &env),
            Ok(Program(Expr::Lit(device::Value::Int(10)), 0))
        );

        assert_eq!(
            Program::compile("{on_time} > 10.0 -> {bulb}", &env),
            Ok(Program(
                Expr::Lt(
                    Box::new(Expr::Lit(device::Value::Flt(10.0))),
                    Box::new(Expr::Var(1))
                ),
                0
            ))
        );

        assert_eq!(
            Program::compile(
                "4 + ({on_time} + 5) * 10 > 10.0 % 3 -> {bulb}",
                &env
            ),
            Ok(Program(
                Expr::Lt(
                    Box::new(Expr::Rem(
                        Box::new(Expr::Lit(device::Value::Flt(10.0))),
                        Box::new(Expr::Lit(device::Value::Int(3)))
                    )),
                    Box::new(Expr::Add(
                        Box::new(Expr::Lit(device::Value::Int(4))),
                        Box::new(Expr::Mul(
                            Box::new(Expr::Add(
                                Box::new(Expr::Var(1)),
                                Box::new(Expr::Lit(device::Value::Int(5)))
                            )),
                            Box::new(Expr::Lit(device::Value::Int(10)))
                        ))
                    ))
                ),
                0
            ))
        );

        assert_eq!(
            Program::compile(
                "true and false or false and true -> {bulb}",
                &env
            ),
            Ok(Program(
                Expr::Or(
                    Box::new(Expr::And(
                        Box::new(Expr::Lit(device::Value::Bool(true))),
                        Box::new(Expr::Lit(device::Value::Bool(false)))
                    )),
                    Box::new(Expr::And(
                        Box::new(Expr::Lit(device::Value::Bool(false))),
                        Box::new(Expr::Lit(device::Value::Bool(true)))
                    ))
                ),
                0
            ))
        );

        assert_eq!(
            Program::compile("\"Hello, world!\" -> {bulb}", &env),
            Ok(Program(
                Expr::Lit(device::Value::Str("Hello, world!".to_string())),
                0
            ))
        );
    }

    #[test]
    fn test_eval_not_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);

        assert_eq!(
            eval(&Expr::Not(Box::new(Expr::Lit(FALSE))), &[]),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::Not(Box::new(Expr::Lit(TRUE))), &[]),
            Some(FALSE)
        );
        assert_eq!(
            eval(&Expr::Not(Box::new(Expr::Lit(device::Value::Int(1)))), &[]),
            None
        );
    }

    #[test]
    fn test_eval_or_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ONE: device::Value = device::Value::Int(1);

        assert_eq!(
            eval(
                &Expr::Or(
                    Box::new(Expr::Lit(FALSE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Or(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Or(
                    Box::new(Expr::Lit(FALSE)),
                    Box::new(Expr::Lit(TRUE))
                ),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(TRUE))),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TRUE))),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Lit(FALSE)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            None
        );
        // This is a loophole for expression errors. If the first
        // subexpression is `true`, we don't evaluate the second so
        // we won't catch type errors until the first subexpression is
        // `false`.
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            Some(TRUE)
        );
    }

    #[test]
    fn test_eval_and_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ONE: device::Value = device::Value::Int(1);

        assert_eq!(
            eval(
                &Expr::And(
                    Box::new(Expr::Lit(FALSE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(
                    Box::new(Expr::Lit(FALSE)),
                    Box::new(Expr::Lit(TRUE))
                ),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(TRUE))
                ),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TRUE))),
                &[]
            ),
            None
        );
        // This is a loophole for expression errors. If the first
        // subexpression is `false`, we don't evaluate the second so
        // we won't catch type errors until the first subexpression is
        // `true`.
        assert_eq!(
            eval(
                &Expr::And(
                    Box::new(Expr::Lit(FALSE)),
                    Box::new(Expr::Lit(ONE))
                ),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            None
        );
    }

    #[test]
    fn test_eval_eq_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ONE: device::Value = device::Value::Int(1);
        const TWO: device::Value = device::Value::Int(2);
        const FP_ONE: device::Value = device::Value::Flt(1.0);

        assert_eq!(
            eval(
                &Expr::Eq(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Eq(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TWO))),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Eq(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Eq(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(ONE))
                ),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Eq(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(FALSE))),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Eq(
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "same"
                    )))),
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "same"
                    ))))
                ),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Eq(
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "same"
                    )))),
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "not same"
                    ))))
                ),
                &[]
            ),
            Some(FALSE)
        );
    }

    #[test]
    fn test_eval_lt_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ONE: device::Value = device::Value::Int(1);
        const TWO: device::Value = device::Value::Int(2);
        const FP_ONE: device::Value = device::Value::Flt(1.0);

        assert_eq!(
            eval(
                &Expr::Lt(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(TWO))),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TWO))),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(FALSE))),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Lt(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(TWO))
                ),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "abc"
                    )))),
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "abc"
                    ))))
                ),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "abc"
                    )))),
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "abcd"
                    ))))
                ),
                &[]
            ),
            Some(TRUE)
        );
    }

    #[test]
    fn test_eval_lteq_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ONE: device::Value = device::Value::Int(1);
        const TWO: device::Value = device::Value::Int(2);
        const FP_ONE: device::Value = device::Value::Flt(1.0);

        assert_eq!(
            eval(
                &Expr::LtEq(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(TWO))),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TWO))),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(TWO))
                ),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "abcd"
                    )))),
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "abc"
                    ))))
                ),
                &[]
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "abc"
                    )))),
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "abc"
                    ))))
                ),
                &[]
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "abc"
                    )))),
                    Box::new(Expr::Lit(device::Value::Str(String::from(
                        "abcd"
                    ))))
                ),
                &[]
            ),
            Some(TRUE)
        );
    }

    #[test]
    fn test_eval_add_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ONE: device::Value = device::Value::Int(1);
        const TWO: device::Value = device::Value::Int(2);
        const FP_ONE: device::Value = device::Value::Flt(1.0);
        const FP_TWO: device::Value = device::Value::Flt(2.0);

        assert_eq!(
            eval(
                &Expr::Add(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            Some(device::Value::Int(3))
        );
        assert_eq!(
            eval(
                &Expr::Add(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            Some(device::Value::Int(2))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(3.0))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(3.0))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(3.0))
        );
    }

    #[test]
    fn test_eval_sub_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ONE: device::Value = device::Value::Int(1);
        const TWO: device::Value = device::Value::Int(2);
        const FP_ONE: device::Value = device::Value::Flt(1.0);
        const FP_TWO: device::Value = device::Value::Flt(2.0);

        assert_eq!(
            eval(
                &Expr::Sub(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            eval(
                &Expr::Sub(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            Some(device::Value::Int(0))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(0.0))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(1.0))
        );
    }

    #[test]
    fn test_eval_mul_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ONE: device::Value = device::Value::Int(1);
        const TWO: device::Value = device::Value::Int(2);
        const FP_ONE: device::Value = device::Value::Flt(1.0);
        const FP_TWO: device::Value = device::Value::Flt(2.0);

        assert_eq!(
            eval(
                &Expr::Mul(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            Some(device::Value::Int(2))
        );
        assert_eq!(
            eval(
                &Expr::Mul(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            Some(device::Value::Int(0))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            Some(device::Value::Flt(0.0))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(2.0))
        );
    }

    #[test]
    fn test_eval_div_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ZERO: device::Value = device::Value::Int(0);
        const ONE: device::Value = device::Value::Int(1);
        const TWO: device::Value = device::Value::Int(2);
        const FP_ZERO: device::Value = device::Value::Flt(0.0);
        const FP_ONE: device::Value = device::Value::Flt(1.0);
        const FP_TWO: device::Value = device::Value::Flt(2.0);

        assert_eq!(
            eval(
                &Expr::Div(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            Some(device::Value::Int(2))
        );
        assert_eq!(
            eval(
                &Expr::Div(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Div(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ZERO))),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ZERO))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ZERO))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ZERO))
                ),
                &[]
            ),
            None
        );
    }

    #[test]
    fn test_eval_rem_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ZERO: device::Value = device::Value::Int(0);
        const NEG_ONE: device::Value = device::Value::Int(-1);
        const ONE: device::Value = device::Value::Int(1);
        const TWO: device::Value = device::Value::Int(2);
        const FP_ZERO: device::Value = device::Value::Flt(0.0);
        const FP_ONE: device::Value = device::Value::Flt(1.0);
        const FP_TWO: device::Value = device::Value::Flt(2.0);

        assert_eq!(
            eval(
                &Expr::Rem(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TWO))),
                &[]
            ),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            eval(
                &Expr::Rem(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FP_TWO))
                ),
                &[]
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(TWO))
                ),
                &[]
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FP_TWO))
                ),
                &[]
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Rem(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ZERO))),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ZERO))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ZERO))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ZERO))
                ),
                &[]
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(NEG_ONE))
                ),
                &[]
            ),
            None
        );
    }

    #[test]
    fn test_eval() {
        const FALSE: device::Value = device::Value::Bool(false);

        assert_eq!(eval(&Expr::Lit(FALSE), &[]), Some(FALSE));
    }

    // This function tests the optimizations that can be done on an
    // expression.

    #[test]
    fn test_not_optimizer() {
        assert_eq!(
            optimize(Expr::Not(Box::new(Expr::Lit(device::Value::Bool(true))))),
            Expr::Lit(device::Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::Not(Box::new(Expr::Lit(device::Value::Bool(
                false
            ))))),
            Expr::Lit(device::Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Not(Box::new(Expr::Not(Box::new(Expr::Lit(
                device::Value::Bool(true)
            )))))),
            Expr::Lit(device::Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Not(Box::new(Expr::Not(Box::new(Expr::Not(
                Box::new(Expr::Lit(device::Value::Bool(true)))
            )))))),
            Expr::Lit(device::Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::Not(Box::new(Expr::Not(Box::new(Expr::Not(
                Box::new(Expr::Not(Box::new(Expr::Lit(device::Value::Bool(
                    true
                )))))
            )))))),
            Expr::Lit(device::Value::Bool(true))
        );
    }

    #[test]
    fn test_and_optimizer() {
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(device::Value::Bool(false))),
                Box::new(Expr::Lit(device::Value::Bool(false)))
            )),
            Expr::Lit(device::Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(device::Value::Bool(false))),
                Box::new(Expr::Lit(device::Value::Bool(true)))
            )),
            Expr::Lit(device::Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(device::Value::Bool(false))),
                Box::new(Expr::Lit(device::Value::Str(String::from("test"))))
            )),
            Expr::Lit(device::Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(device::Value::Bool(true))),
                Box::new(Expr::Lit(device::Value::Bool(false)))
            )),
            Expr::Lit(device::Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(device::Value::Bool(true))),
                Box::new(Expr::Lit(device::Value::Bool(true)))
            )),
            Expr::Lit(device::Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(device::Value::Bool(true))),
                Box::new(Expr::Lit(device::Value::Str(String::from("test"))))
            )),
            Expr::Lit(device::Value::Str(String::from("test")))
        );

        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(device::Value::Bool(true))),
                Box::new(Expr::And(
                    Box::new(Expr::Lit(device::Value::Bool(true))),
                    Box::new(Expr::Lit(device::Value::Bool(true)))
                ))
            )),
            Expr::Lit(device::Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::And(
                    Box::new(Expr::Lit(device::Value::Bool(true))),
                    Box::new(Expr::Lit(device::Value::Bool(true)))
                )),
                Box::new(Expr::Lit(device::Value::Bool(true)))
            )),
            Expr::Lit(device::Value::Bool(true))
        );

        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::Lit(device::Value::Bool(true))),
                Box::new(Expr::And(
                    Box::new(Expr::Lit(device::Value::Bool(true))),
                    Box::new(Expr::Lit(device::Value::Bool(false)))
                ))
            )),
            Expr::Lit(device::Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::And(
                Box::new(Expr::And(
                    Box::new(Expr::Lit(device::Value::Bool(true))),
                    Box::new(Expr::Lit(device::Value::Bool(true)))
                )),
                Box::new(Expr::Lit(device::Value::Bool(false)))
            )),
            Expr::Lit(device::Value::Bool(false))
        );
    }

    #[test]
    fn test_or_optimizer() {
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(device::Value::Bool(false))),
                Box::new(Expr::Lit(device::Value::Bool(false)))
            )),
            Expr::Lit(device::Value::Bool(false))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(device::Value::Bool(false))),
                Box::new(Expr::Lit(device::Value::Bool(true)))
            )),
            Expr::Lit(device::Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(device::Value::Bool(false))),
                Box::new(Expr::Lit(device::Value::Str(String::from("test"))))
            )),
            Expr::Lit(device::Value::Str(String::from("test")))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(device::Value::Bool(true))),
                Box::new(Expr::Lit(device::Value::Bool(false)))
            )),
            Expr::Lit(device::Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(device::Value::Bool(true))),
                Box::new(Expr::Lit(device::Value::Bool(true)))
            )),
            Expr::Lit(device::Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(device::Value::Bool(true))),
                Box::new(Expr::Lit(device::Value::Str(String::from("test"))))
            )),
            Expr::Lit(device::Value::Bool(true))
        );

        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(device::Value::Bool(true))),
                Box::new(Expr::Or(
                    Box::new(Expr::Lit(device::Value::Bool(true))),
                    Box::new(Expr::Lit(device::Value::Bool(true)))
                ))
            )),
            Expr::Lit(device::Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Or(
                    Box::new(Expr::Lit(device::Value::Bool(true))),
                    Box::new(Expr::Lit(device::Value::Bool(true)))
                )),
                Box::new(Expr::Lit(device::Value::Bool(true)))
            )),
            Expr::Lit(device::Value::Bool(true))
        );

        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Lit(device::Value::Bool(true))),
                Box::new(Expr::Or(
                    Box::new(Expr::Lit(device::Value::Bool(true))),
                    Box::new(Expr::Lit(device::Value::Bool(false)))
                ))
            )),
            Expr::Lit(device::Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Or(
                    Box::new(Expr::Lit(device::Value::Bool(true))),
                    Box::new(Expr::Lit(device::Value::Bool(true)))
                )),
                Box::new(Expr::Lit(device::Value::Bool(false)))
            )),
            Expr::Lit(device::Value::Bool(true))
        );
        assert_eq!(
            optimize(Expr::Or(
                Box::new(Expr::Or(
                    Box::new(Expr::Lit(device::Value::Bool(false))),
                    Box::new(Expr::Lit(device::Value::Bool(false)))
                )),
                Box::new(Expr::Lit(device::Value::Bool(false)))
            )),
            Expr::Lit(device::Value::Bool(false))
        );
    }

    #[test]
    fn test_to_string() {
        let env: Env = (
            &[String::from("a"), String::from("b")],
            &[String::from("b"), String::from("c")],
        );

        assert_eq!(
            Program::compile("{a} -> {b}", &env).unwrap().to_string(),
            "inp[0] -> out[0]"
        );

        assert_eq!(
            Program::compile("true -> {b}", &env).unwrap().to_string(),
            "true -> out[0]"
        );
        assert_eq!(
            Program::compile("not true -> {b}", &env)
                .unwrap()
                .to_string(),
            "not (true) -> out[0]"
        );
        assert_eq!(
            Program::compile("{a} and {b} -> {c}", &env)
                .unwrap()
                .to_string(),
            "(inp[0]) and (inp[1]) -> out[1]"
        );
        assert_eq!(
            Program::compile("{a} or {b} -> {c}", &env)
                .unwrap()
                .to_string(),
            "(inp[0]) or (inp[1]) -> out[1]"
        );
        assert_eq!(
            Program::compile("{a} = {b} -> {c}", &env)
                .unwrap()
                .to_string(),
            "(inp[0]) = (inp[1]) -> out[1]"
        );
        assert_eq!(
            Program::compile("{a} < {b} -> {c}", &env)
                .unwrap()
                .to_string(),
            "(inp[0]) < (inp[1]) -> out[1]"
        );
        assert_eq!(
            Program::compile("{a} <= {b} -> {c}", &env)
                .unwrap()
                .to_string(),
            "(inp[0]) <= (inp[1]) -> out[1]"
        );
        assert_eq!(
            Program::compile("{a} + {b} -> {c}", &env)
                .unwrap()
                .to_string(),
            "(inp[0]) + (inp[1]) -> out[1]"
        );
        assert_eq!(
            Program::compile("{a} - {b} -> {c}", &env)
                .unwrap()
                .to_string(),
            "(inp[0]) - (inp[1]) -> out[1]"
        );
        assert_eq!(
            Program::compile("{a} * {b} -> {c}", &env)
                .unwrap()
                .to_string(),
            "(inp[0]) * (inp[1]) -> out[1]"
        );
        assert_eq!(
            Program::compile("{a} / {b} -> {c}", &env)
                .unwrap()
                .to_string(),
            "(inp[0]) / (inp[1]) -> out[1]"
        );
        assert_eq!(
            Program::compile("{a} % {b} -> {c}", &env)
                .unwrap()
                .to_string(),
            "(inp[0]) % (inp[1]) -> out[1]"
        );

        assert_eq!(
            Program::compile("{a} * 3 + {b} > 4 -> {c}", &env)
                .unwrap()
                .to_string(),
            "(4) < (((inp[0]) * (3)) + (inp[1])) -> out[1]"
        );
    }

    fn evaluate(expr: &str) -> Option<device::Value> {
        let env: Env = (&[], &[String::from("a")]);
        let expr = format!("{} -> {{a}}", expr);
        let prog = Program::compile(&expr, &env).unwrap();

        eval(&prog.0, &[])
    }

    #[test]
    fn test_evaluations() {
        assert_eq!(evaluate("1 / 0"), None);
        assert_eq!(evaluate("5 > true"), None);

        assert_eq!(evaluate("1 + 2 * 3"), Some(device::Value::Int(7)));
        assert_eq!(evaluate("(1 + 2) * 3"), Some(device::Value::Int(9)));
        assert_eq!(evaluate("1 + (2 * 3)"), Some(device::Value::Int(7)));

        assert_eq!(evaluate("1 + 2 < 1 + 3"), Some(device::Value::Bool(true)));
        assert_eq!(evaluate("1 + 2 < 1 + 1"), Some(device::Value::Bool(false)));

        assert_eq!(
            evaluate("1 > 2 or 5 < 3"),
            Some(device::Value::Bool(false))
        );
        assert_eq!(
            evaluate("1 > 2 or 5 >= 3"),
            Some(device::Value::Bool(true))
        );
    }
}
