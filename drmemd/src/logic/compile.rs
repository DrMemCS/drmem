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
//     #rrggbb or
//     #name		 RGB color values
//
// There are two built-in types, "utc" and "local", which can be used
// to obtain time-of-day values. Use the {} notation to access them:
//
//     {utc:second}
//     {utc:minute}
//     {utc:hour}
//     {utc:day}
//     {utc:month}
//     {utc:year}
//     {utc:DOW}	day of week (Monday = 0, Sunday = 6)
//     {utc:DOY}	day of year from 0 to 365
//     {utc:LY}		true if it's a leap year
//
//     {local:second}
//     {local:minute}
//     {local:hour}
//     {local:day}
//     {local:month}
//     {local:year}
//     {local:DOW}	day of week (Monday = 0, Sunday = 6)
//     {local:DOY}	day of year from 0 to 365
//     {local:LY}	true if it's a leap year
//
// There is a built-in type, "solar", that provides solar position in
// the sky.
//
//     {solar:alt}	altitude of sun (< 0 is below horizon, > 0 is above)
//     {solar:az}	azimuth of sun
//     {solar:ra}	right ascension of sun
//     {solar:dec}	declination of sun
//
// The token "->" represents assignment. The only item that can be on
// the right hand side of the arrow is a variable referring to a
// settable device (for logic blocks, output devices are specified in
// the `output` map of the configuration).
//
// Parentheses can be used to group subexpressions.
//
//     not EXPR          Computes the complement of a boolean expression
//     EXPR or EXPR      Computes the boolean OR of two boolean expressions
//     EXPR and EXPR     Computes the boolean AND of two boolean expressions
//
//     =,<>,<,<=,>,>=    Perform the comparison and return a boolean
//
//     +,-,*,/,%         Perform addition, subtraction, multiplication,
//                       division, and modulo operations

use super::solar;
use super::tod;
use drmem_api::{device, Error, Result};
use lrlex::lrlex_mod;
use lrpar::lrpar_mod;
use std::fmt;
use tracing::error;

// Pull in the lexer and parser for the Logic Node language.

lrlex_mod!("logic/logic.l");
lrpar_mod!("logic/logic.y");

#[derive(Clone, PartialEq, Debug)]
pub enum TimeField {
    Second,
    Minute,
    Hour,
    Day,
    DoW,
    DoY,
    Month,
    Year,
    LeapYear,
}

impl std::fmt::Display for TimeField {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::result::Result<(), std::fmt::Error> {
        match self {
            TimeField::Second => write!(f, "second"),
            TimeField::Minute => write!(f, "minute"),
            TimeField::Hour => write!(f, "hour"),
            TimeField::Day => write!(f, "day"),
            TimeField::DoW => write!(f, "DOW"),
            TimeField::Month => write!(f, "month"),
            TimeField::Year => write!(f, "year"),
            TimeField::DoY => write!(f, "DOY"),
            TimeField::LeapYear => write!(f, "LY"),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum SolarField {
    Elevation,
    Azimuth,
    RightAscension,
    Declination,
}

impl std::fmt::Display for SolarField {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::result::Result<(), std::fmt::Error> {
        match self {
            SolarField::Elevation => write!(f, "alt"),
            SolarField::Azimuth => write!(f, "az"),
            SolarField::RightAscension => write!(f, "ra"),
            SolarField::Declination => write!(f, "dec"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Lit(device::Value),
    Var(usize),
    TimeVal(&'static str, TimeField, fn(&tod::Info) -> device::Value),
    SolarVal(SolarField, fn(&solar::Info) -> device::Value),

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

impl Expr {
    pub fn precedence(&self) -> u32 {
        match self {
            Expr::Lit(_)
            | Expr::Var(_)
            | Expr::TimeVal(..)
            | Expr::SolarVal(..) => 10,
            Expr::Not(_) => 9,
            Expr::Mul(_, _) | Expr::Div(_, _) | Expr::Rem(_, _) => 5,
            Expr::Add(_, _) | Expr::Sub(_, _) => 4,
            Expr::Lt(_, _) | Expr::LtEq(_, _) | Expr::Eq(_, _) => 3,
            Expr::And(_, _) => 2,
            Expr::Or(_, _) => 1,
        }
    }

    // Traverses an expression and returns `true` if it uses any
    // `TimeVal()` variants.

    pub fn uses_time(&self) -> Option<tod::TimeField> {
        match self {
            Expr::TimeVal(_, TimeField::Second, _) => {
                Some(tod::TimeField::Second)
            }
            Expr::TimeVal(_, TimeField::Minute, _) => {
                Some(tod::TimeField::Minute)
            }
            Expr::TimeVal(_, TimeField::Hour, _) => Some(tod::TimeField::Hour),
            Expr::TimeVal(_, TimeField::Day, _)
            | Expr::TimeVal(_, TimeField::DoW, _)
            | Expr::TimeVal(_, TimeField::DoY, _) => Some(tod::TimeField::Day),
            Expr::TimeVal(_, TimeField::Month, _) => {
                Some(tod::TimeField::Month)
            }
            Expr::TimeVal(_, TimeField::Year, _)
            | Expr::TimeVal(_, TimeField::LeapYear, _) => {
                Some(tod::TimeField::Year)
            }
            Expr::SolarVal(..) | Expr::Lit(_) | Expr::Var(_) => None,
            Expr::Not(e) => e.uses_time(),
            Expr::Mul(a, b)
            | Expr::Div(a, b)
            | Expr::Rem(a, b)
            | Expr::Add(a, b)
            | Expr::Sub(a, b)
            | Expr::Lt(a, b)
            | Expr::LtEq(a, b)
            | Expr::Eq(a, b)
            | Expr::And(a, b)
            | Expr::Or(a, b) => match (a.uses_time(), b.uses_time()) {
                (None, None) => None,
                (a, None) => a,
                (None, b) => b,
                (Some(a), Some(b)) => Some(a.min(b)),
            },
        }
    }

    // Traverses an expression and returns `true` if it uses any
    // `SolarVal()` variants.

    pub fn uses_solar(&self) -> bool {
        match self {
            Expr::SolarVal(..) => true,
            Expr::TimeVal(..) | Expr::Lit(_) | Expr::Var(_) => false,
            Expr::Not(e) => e.uses_solar(),
            Expr::Mul(a, b)
            | Expr::Div(a, b)
            | Expr::Rem(a, b)
            | Expr::Add(a, b)
            | Expr::Sub(a, b)
            | Expr::Lt(a, b)
            | Expr::LtEq(a, b)
            | Expr::Eq(a, b)
            | Expr::And(a, b)
            | Expr::Or(a, b) => a.uses_solar() || b.uses_solar(),
        }
    }

    fn fmt_subexpr(&self, e: &Expr, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let my_prec = self.precedence();

        if my_prec > e.precedence() {
            write!(f, "({})", &e)
        } else {
            write!(f, "{}", &e)
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Lit(v) => write!(f, "{}", &v),
            Expr::Var(v) => write!(f, "inp[{}]", &v),

            Expr::TimeVal(cat, fld, _) => write!(f, "{{{}:{}}}", cat, fld),

            Expr::SolarVal(fld, _) => write!(f, "{{solar:{}}}", fld),

            Expr::Not(e) => {
                write!(f, "not ")?;
                self.fmt_subexpr(e, f)
            }

            Expr::And(a, b) => {
                self.fmt_subexpr(a, f)?;
                write!(f, " and ")?;
                self.fmt_subexpr(b, f)
            }

            Expr::Or(a, b) => {
                self.fmt_subexpr(a, f)?;
                write!(f, " or ")?;
                self.fmt_subexpr(b, f)
            }

            Expr::Eq(a, b) => {
                self.fmt_subexpr(a, f)?;
                write!(f, " = ")?;
                self.fmt_subexpr(b, f)
            }

            Expr::Lt(a, b) => {
                self.fmt_subexpr(a, f)?;
                write!(f, " < ")?;
                self.fmt_subexpr(b, f)
            }

            Expr::LtEq(a, b) => {
                self.fmt_subexpr(a, f)?;
                write!(f, " <= ")?;
                self.fmt_subexpr(b, f)
            }

            Expr::Add(a, b) => {
                self.fmt_subexpr(a, f)?;
                write!(f, " + ")?;
                self.fmt_subexpr(b, f)
            }

            Expr::Sub(a, b) => {
                self.fmt_subexpr(a, f)?;
                write!(f, " - ")?;
                self.fmt_subexpr(b, f)
            }

            Expr::Mul(a, b) => {
                self.fmt_subexpr(a, f)?;
                write!(f, " * ")?;
                self.fmt_subexpr(b, f)
            }

            Expr::Div(a, b) => {
                self.fmt_subexpr(a, f)?;
                write!(f, " / ")?;
                self.fmt_subexpr(b, f)
            }

            Expr::Rem(a, b) => {
                self.fmt_subexpr(a, f)?;
                write!(f, " % ")?;
                self.fmt_subexpr(b, f)
            }
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
            let res = errs.iter().fold(s.to_owned(), |mut acc, e| {
                acc.push_str(&format!("\n    {}", &e));
                acc
            });

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

pub fn eval(
    e: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match e {
        // Literals hold actual `device::Values`, so simply return it.
        Expr::Lit(v) => Some(v.clone()),

        Expr::Var(n) => eval_as_var(*n, inp),

        Expr::TimeVal(_, _, f) => Some(f(time)),

        Expr::SolarVal(_, f) => solar.map(f),

        Expr::Not(ref e) => eval_as_not_expr(e, inp, time, solar),

        Expr::Or(ref a, ref b) => eval_as_or_expr(a, b, inp, time, solar),

        Expr::And(ref a, ref b) => eval_as_and_expr(a, b, inp, time, solar),

        Expr::Eq(ref a, ref b) => eval_as_eq_expr(a, b, inp, time, solar),

        Expr::Lt(ref a, ref b) => eval_as_lt_expr(a, b, inp, time, solar),

        Expr::LtEq(ref a, ref b) => eval_as_lteq_expr(a, b, inp, time, solar),

        Expr::Add(ref a, ref b) => eval_as_add_expr(a, b, inp, time, solar),

        Expr::Sub(ref a, ref b) => eval_as_sub_expr(a, b, inp, time, solar),

        Expr::Mul(ref a, ref b) => eval_as_mul_expr(a, b, inp, time, solar),

        Expr::Div(ref a, ref b) => eval_as_div_expr(a, b, inp, time, solar),

        Expr::Rem(ref a, ref b) => eval_as_rem_expr(a, b, inp, time, solar),
    }
}

// Returns the latest value of the variable.

fn eval_as_var(
    idx: usize,
    inp: &[Option<device::Value>],
) -> Option<device::Value> {
    inp[idx].clone()
}

// Evaluates the subexpression of a NOT expression. It only accepts
// booleans as values and simply complements the value.

fn eval_as_not_expr(
    e: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match eval(e, inp, time, solar) {
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
    a: &Expr,
    b: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match eval(a, inp, time, solar) {
        v @ Some(device::Value::Bool(true)) => v,
        Some(device::Value::Bool(false)) => match eval(b, inp, time, solar) {
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
        None => match eval(b, inp, time, solar) {
            v @ Some(device::Value::Bool(true)) => v,
            Some(device::Value::Bool(false)) | None => None,
            Some(v) => {
                error!("OR expression contains non-boolean argument: {}", &v);
                None
            }
        },
    }
}

// AND expressions. If the first subexpression is `false`, the second
// subexpression isn't evaluated.
fn eval_as_and_expr(
    a: &Expr,
    b: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match eval(a, inp, time, solar) {
        v @ Some(device::Value::Bool(false)) => v,
        Some(device::Value::Bool(true)) => match eval(b, inp, time, solar) {
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
        None => match eval(b, inp, time, solar) {
            v @ Some(device::Value::Bool(false)) => v,
            Some(device::Value::Bool(true)) | None => None,
            Some(v) => {
                error!("AND expression contains non-boolean argument: {}", &v);
                None
            }
        },
    }
}

// EQ expressions. Both expressions must be of the same type.
fn eval_as_eq_expr(
    a: &Expr,
    b: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match (eval(a, inp, time, solar), eval(b, inp, time, solar)) {
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
    a: &Expr,
    b: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match (eval(a, inp, time, solar), eval(b, inp, time, solar)) {
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
    a: &Expr,
    b: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match (eval(a, inp, time, solar), eval(b, inp, time, solar)) {
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
    a: &Expr,
    b: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match (eval(a, inp, time, solar), eval(b, inp, time, solar)) {
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
    a: &Expr,
    b: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match (eval(a, inp, time, solar), eval(b, inp, time, solar)) {
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
    a: &Expr,
    b: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match (eval(a, inp, time, solar), eval(b, inp, time, solar)) {
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
    a: &Expr,
    b: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match (eval(a, inp, time, solar), eval(b, inp, time, solar)) {
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
    a: &Expr,
    b: &Expr,
    inp: &[Option<device::Value>],
    time: &tod::Info,
    solar: Option<&solar::Info>,
) -> Option<device::Value> {
    match (eval(a, inp, time, solar), eval(b, inp, time, solar)) {
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
    use palette::LinSrgba;
    use std::sync::Arc;

    fn to_expr(expr: &str) -> Expr {
        let env: Env = (
            &[String::from("a"), String::from("b")],
            &[String::from("c")],
        );

        match Program::compile(&format!("{} -> {{c}}", expr), &env) {
            Ok(Program(expr, _)) => expr,
            Err(_) => panic!("couldn't parse {}", expr),
        }
    }

    #[test]
    fn test_parser() {
        let env: Env = (
            &[String::from("switch"), String::from("on_time")],
            &[String::from("bulb")],
        );

        assert!(Program::compile("", &env).is_err());
        assert!(Program::compile("{switch -> {bulb}", &env).is_err());
        assert!(Program::compile("switch} -> {bulb}", &env).is_err());

        // Test for defined categories and fields.

        assert!(Program::compile("{utc:second} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{utc:minute} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{utc:hour} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{utc:day} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{utc:month} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{utc:year} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{utc:DOW} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{utc:DOY} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{utc:LY} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{local:second} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{local:minute} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{local:hour} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{local:day} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{local:month} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{local:year} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{local:DOW} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{local:DOY} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{local:LY} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{solar:alt} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{solar:az} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{solar:ra} -> {bulb}", &env).is_ok());
        assert!(Program::compile("{solar:dec} -> {bulb}", &env).is_ok());

        // Don't allow bad categories or fields.

        assert!(Program::compile("{bad:second} -> {bulb}", &env).is_err());
        assert!(Program::compile("{utc:bad} -> {bulb}", &env).is_err());
        assert!(Program::compile("{local:bad} -> {bulb}", &env).is_err());

        // Don't allow whitespace.

        assert!(Program::compile("{ switch} -> {bulb}", &env).is_err());
        assert!(Program::compile("{switch } -> {bulb}", &env).is_err());
        assert!(Program::compile("{ utc:second} -> {bulb}", &env).is_err());
        assert!(Program::compile("{utc :second} -> {bulb}", &env).is_err());
        assert!(Program::compile("{utc: second} -> {bulb}", &env).is_err());
        assert!(Program::compile("{utc:second } -> {bulb}", &env).is_err());

        // Test proper compilations.

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

        assert!(Program::compile("#1 -> {bulb}", &env).is_err());
        assert!(Program::compile("#12 -> {bulb}", &env).is_err());
        assert!(Program::compile("#12345 -> {bulb}", &env).is_err());
        assert_eq!(
            Program::compile("#123 -> {bulb}", &env),
            Ok(Program(
                Expr::Lit(device::Value::Color(LinSrgba::new(
                    0x11, 0x22, 0x33, 255
                ))),
                0
            ))
        );
        assert_eq!(
            Program::compile("#1234 -> {bulb}", &env),
            Ok(Program(
                Expr::Lit(device::Value::Color(LinSrgba::new(
                    0x11, 0x22, 0x33, 0x44
                ))),
                0
            ))
        );
        assert_eq!(
            Program::compile("#7f8081 -> {bulb}", &env),
            Ok(Program(
                Expr::Lit(device::Value::Color(LinSrgba::new(
                    127, 128, 129, 255
                ))),
                0
            ))
        );
        assert_eq!(
            Program::compile("#7f808182 -> {bulb}", &env),
            Ok(Program(
                Expr::Lit(device::Value::Color(LinSrgba::new(
                    127, 128, 129, 130
                ))),
                0
            ))
        );
        assert_eq!(
            Program::compile("#7F80A0 -> {bulb}", &env),
            Ok(Program(
                Expr::Lit(device::Value::Color(LinSrgba::new(
                    127, 128, 160, 255
                ))),
                0
            ))
        );
        assert_eq!(
            Program::compile("#black -> {bulb}", &env),
            Ok(Program(
                Expr::Lit(device::Value::Color(LinSrgba::new(0, 0, 0, 255))),
                0
            ))
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
            Program::compile("true and (5 < 7 or true) -> {bulb}", &env),
            Ok(Program(
                Expr::And(
                    Box::new(Expr::Lit(device::Value::Bool(true))),
                    Box::new(Expr::Or(
                        Box::new(Expr::Lt(
                            Box::new(Expr::Lit(device::Value::Int(5))),
                            Box::new(Expr::Lit(device::Value::Int(7)))
                        )),
                        Box::new(Expr::Lit(device::Value::Bool(true)))
                    ))
                ),
                0
            ))
        );

        assert_eq!(
            Program::compile("\"Hello, world!\" -> {bulb}", &env),
            Ok(Program(
                Expr::Lit(device::Value::Str("Hello, world!".into())),
                0
            ))
        );
    }

    #[test]
    fn test_eval_not_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        // Test for uninitialized and initialized variables.

        assert_eq!(
            eval(&Expr::Not(Box::new(Expr::Var(0))), &[None], &time, None),
            None
        );
        assert_eq!(
            eval(
                &Expr::Not(Box::new(Expr::Var(0))),
                &[Some(device::Value::Bool(true))],
                &time,
                None
            ),
            Some(device::Value::Bool(false))
        );

        // Test literal values.

        assert_eq!(
            eval(&Expr::Not(Box::new(Expr::Lit(FALSE))), &[], &time, None),
            Some(TRUE)
        );
        assert_eq!(
            eval(&Expr::Not(Box::new(Expr::Lit(TRUE))), &[], &time, None),
            Some(FALSE)
        );

        // Test incorrect types.

        assert_eq!(
            eval(
                &Expr::Not(Box::new(Expr::Lit(device::Value::Int(1)))),
                &[],
                &time,
                None
            ),
            None
        );
    }

    #[test]
    fn test_eval_or_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ONE: device::Value = device::Value::Int(1);
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        // Test uninitialized and initialized variables.

        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[Some(FALSE), Some(FALSE)],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[Some(FALSE), Some(TRUE)],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[Some(TRUE), Some(FALSE)],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[Some(TRUE), None],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[Some(FALSE), None],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[None, Some(TRUE)],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[None, Some(FALSE)],
                &time,
                None
            ),
            None
        );

        // Test literal values.

        assert_eq!(
            eval(
                &Expr::Or(
                    Box::new(Expr::Lit(FALSE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Or(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Or(
                    Box::new(Expr::Lit(FALSE)),
                    Box::new(Expr::Lit(TRUE))
                ),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(TRUE))),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );

        // Test invalid types.

        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TRUE))),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Or(Box::new(Expr::Lit(FALSE)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
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
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
    }

    #[test]
    fn test_eval_and_expr() {
        const TRUE: device::Value = device::Value::Bool(true);
        const FALSE: device::Value = device::Value::Bool(false);
        const ONE: device::Value = device::Value::Int(1);
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        // Test uninitialized and initialized variables.

        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[Some(FALSE), Some(FALSE)],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[Some(FALSE), Some(TRUE)],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[Some(TRUE), Some(FALSE)],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[Some(TRUE), Some(TRUE)],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[Some(TRUE), None],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[Some(FALSE), None],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[None, Some(TRUE)],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Var(0)), Box::new(Expr::Var(1))),
                &[None, Some(FALSE)],
                &time,
                None
            ),
            Some(FALSE)
        );

        // Test literal values.

        assert_eq!(
            eval(
                &Expr::And(
                    Box::new(Expr::Lit(FALSE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(
                    Box::new(Expr::Lit(FALSE)),
                    Box::new(Expr::Lit(TRUE))
                ),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(TRUE))
                ),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TRUE))),
                &[],
                &time,
                None
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
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::And(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
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
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        assert_eq!(
            eval(
                &Expr::Eq(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Eq(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TWO))),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Eq(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Eq(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Eq(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(FALSE))),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Eq(
                    Box::new(Expr::Lit(device::Value::Str("same".into()))),
                    Box::new(Expr::Lit(device::Value::Str("same".into())))
                ),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Eq(
                    Box::new(Expr::Lit(device::Value::Str("same".into()))),
                    Box::new(Expr::Lit(device::Value::Str("not same".into())))
                ),
                &[],
                &time,
                None
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
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        assert_eq!(
            eval(
                &Expr::Lt(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(TWO))),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TWO))),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(FALSE))),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Lt(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(TWO))
                ),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(
                    Box::new(Expr::Lit(device::Value::Str("abc".into()))),
                    Box::new(Expr::Lit(device::Value::Str("abc".into())))
                ),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::Lt(
                    Box::new(Expr::Lit(device::Value::Str("abc".into()))),
                    Box::new(Expr::Lit(device::Value::Str("abcd".into())))
                ),
                &[],
                &time,
                None
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
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        assert_eq!(
            eval(
                &Expr::LtEq(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(TWO))),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TWO))),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(TWO))
                ),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(device::Value::Str("abcd".into()))),
                    Box::new(Expr::Lit(device::Value::Str("abc".into())))
                ),
                &[],
                &time,
                None
            ),
            Some(FALSE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(device::Value::Str("abc".into()))),
                    Box::new(Expr::Lit(device::Value::Str("abc".into())))
                ),
                &[],
                &time,
                None
            ),
            Some(TRUE)
        );
        assert_eq!(
            eval(
                &Expr::LtEq(
                    Box::new(Expr::Lit(device::Value::Str("abc".into()))),
                    Box::new(Expr::Lit(device::Value::Str("abcd".into())))
                ),
                &[],
                &time,
                None
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
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        assert_eq!(
            eval(
                &Expr::Add(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            Some(device::Value::Int(3))
        );
        assert_eq!(
            eval(
                &Expr::Add(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            Some(device::Value::Int(2))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(3.0))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(3.0))
        );
        assert_eq!(
            eval(
                &Expr::Add(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
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
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        assert_eq!(
            eval(
                &Expr::Sub(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            eval(
                &Expr::Sub(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            Some(device::Value::Int(0))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(0.0))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Sub(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
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
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        assert_eq!(
            eval(
                &Expr::Mul(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            Some(device::Value::Int(2))
        );
        assert_eq!(
            eval(
                &Expr::Mul(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Int(0))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(0.0))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Mul(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
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
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        assert_eq!(
            eval(
                &Expr::Div(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            Some(device::Value::Int(2))
        );
        assert_eq!(
            eval(
                &Expr::Div(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            eval(
                &Expr::Div(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ZERO))),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ZERO))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ZERO))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Div(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ZERO))
                ),
                &[],
                &time,
                None
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
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        assert_eq!(
            eval(
                &Expr::Rem(Box::new(Expr::Lit(ONE)), Box::new(Expr::Lit(TWO))),
                &[],
                &time,
                None
            ),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            eval(
                &Expr::Rem(Box::new(Expr::Lit(TRUE)), Box::new(Expr::Lit(ONE))),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FP_TWO))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(TRUE)),
                    Box::new(Expr::Lit(FP_ONE))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(FALSE))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(FP_ONE)),
                    Box::new(Expr::Lit(TWO))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(ONE)),
                    Box::new(Expr::Lit(FP_TWO))
                ),
                &[],
                &time,
                None
            ),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            eval(
                &Expr::Rem(Box::new(Expr::Lit(TWO)), Box::new(Expr::Lit(ZERO))),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(FP_ZERO))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(ZERO))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(FP_TWO)),
                    Box::new(Expr::Lit(FP_ZERO))
                ),
                &[],
                &time,
                None
            ),
            None
        );
        assert_eq!(
            eval(
                &Expr::Rem(
                    Box::new(Expr::Lit(TWO)),
                    Box::new(Expr::Lit(NEG_ONE))
                ),
                &[],
                &time,
                None
            ),
            None
        );
    }

    #[test]
    fn test_eval() {
        const FALSE: device::Value = device::Value::Bool(false);
        let time = Arc::new((chrono::Utc::now(), chrono::Local::now()));

        assert_eq!(eval(&Expr::Lit(FALSE), &[], &time, None), Some(FALSE));
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
                Box::new(Expr::Lit(device::Value::Str("test".into())))
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
                Box::new(Expr::Lit(device::Value::Str("test".into())))
            )),
            Expr::Lit(device::Value::Str("test".into()))
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
                Box::new(Expr::Lit(device::Value::Str("test".into())))
            )),
            Expr::Lit(device::Value::Str("test".into()))
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
                Box::new(Expr::Lit(device::Value::Str("test".into())))
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

        const TESTS: &[(&str, &str)] = &[
            ("{a} -> {b}", "inp[0] -> out[0]"),
            ("true -> {b}", "true -> out[0]"),
            ("not true -> {b}", "not true -> out[0]"),
            ("{a} and {b} -> {c}", "inp[0] and inp[1] -> out[1]"),
            ("{a} or {b} -> {c}", "inp[0] or inp[1] -> out[1]"),
            (
                "{a} and {b} or true -> {c}",
                "inp[0] and inp[1] or true -> out[1]",
            ),
            (
                "{a} and ({b} or true) -> {c}",
                "inp[0] and (inp[1] or true) -> out[1]",
            ),
            ("{a} = {b} -> {c}", "inp[0] = inp[1] -> out[1]"),
            ("{a} < {b} -> {c}", "inp[0] < inp[1] -> out[1]"),
            ("{a} <= {b} -> {c}", "inp[0] <= inp[1] -> out[1]"),
            ("{a} + {b} -> {c}", "inp[0] + inp[1] -> out[1]"),
            (
                "{a} + {b} + {b} -> {c}",
                "inp[0] + inp[1] + inp[1] -> out[1]",
            ),
            ("{a} - {b} -> {c}", "inp[0] - inp[1] -> out[1]"),
            ("{a} * {b} -> {c}", "inp[0] * inp[1] -> out[1]"),
            ("{a} / {b} -> {c}", "inp[0] / inp[1] -> out[1]"),
            ("{a} % {b} -> {c}", "inp[0] % inp[1] -> out[1]"),
            (
                "{a} * 3 + {b} > 4 -> {c}",
                "4 < inp[0] * 3 + inp[1] -> out[1]",
            ),
            (
                "{a} * (3 + {b}) > 4 -> {c}",
                "4 < inp[0] * (3 + inp[1]) -> out[1]",
            ),
            ("{utc:second} -> {c}", "{utc:second} -> out[1]"),
            ("{utc:minute} -> {c}", "{utc:minute} -> out[1]"),
            ("{utc:hour} -> {c}", "{utc:hour} -> out[1]"),
            ("{utc:day} -> {c}", "{utc:day} -> out[1]"),
            ("{utc:month} -> {c}", "{utc:month} -> out[1]"),
            ("{utc:year} -> {c}", "{utc:year} -> out[1]"),
            ("{utc:DOW} -> {c}", "{utc:DOW} -> out[1]"),
            ("{utc:DOY} -> {c}", "{utc:DOY} -> out[1]"),
            ("{local:second} -> {c}", "{local:second} -> out[1]"),
            ("{local:minute} -> {c}", "{local:minute} -> out[1]"),
            ("{local:hour} -> {c}", "{local:hour} -> out[1]"),
            ("{local:day} -> {c}", "{local:day} -> out[1]"),
            ("{local:month} -> {c}", "{local:month} -> out[1]"),
            ("{local:year} -> {c}", "{local:year} -> out[1]"),
            ("{local:DOW} -> {c}", "{local:DOW} -> out[1]"),
            ("{local:DOY} -> {c}", "{local:DOY} -> out[1]"),
        ];

        for (in_val, out_val) in TESTS {
            match Program::compile(in_val, &env) {
                Ok(prog) => assert_eq!(
                    prog.to_string(),
                    *out_val,
                    "failed on: {}",
                    in_val
                ),
                Err(e) => panic!("{}", &e),
            }
        }
    }

    fn evaluate(
        expr: &str,
        time: &tod::Info,
        solar: Option<&solar::Info>,
    ) -> Option<device::Value> {
        let env: Env = (&[], &[String::from("a")]);
        let expr = format!("{} -> {{a}}", expr);
        let prog = Program::compile(&expr, &env).unwrap();

        eval(&prog.0, &[], &time, solar)
    }

    #[test]
    fn test_evaluations() {
        use chrono::TimeZone;

        let time = Arc::new((
            chrono::Utc
                .with_ymd_and_hms(2000, 1, 2, 3, 4, 5)
                .single()
                .unwrap(),
            chrono::Local
                .with_ymd_and_hms(2001, 6, 7, 8, 9, 10)
                .single()
                .unwrap(),
        ));

        let solar = Arc::new(solar::SolarInfo {
            elevation: 1.0,
            azimuth: 2.0,
            right_ascension: 3.0,
            declination: 4.0,
        });

        assert_eq!(evaluate("1 / 0", &time, None), None);
        assert_eq!(evaluate("5 > true", &time, None), None);

        assert_eq!(
            evaluate("1 + 2 * 3", &time, None),
            Some(device::Value::Int(7))
        );
        assert_eq!(
            evaluate("(1 + 2) * 3", &time, None),
            Some(device::Value::Int(9))
        );
        assert_eq!(
            evaluate("1 + (2 * 3)", &time, None),
            Some(device::Value::Int(7))
        );

        assert_eq!(
            evaluate("1 + 2 < 1 + 3", &time, None),
            Some(device::Value::Bool(true))
        );
        assert_eq!(
            evaluate("1 + 2 < 1 + 1", &time, None),
            Some(device::Value::Bool(false))
        );

        assert_eq!(
            evaluate("1 > 2 or 5 < 3", &time, None),
            Some(device::Value::Bool(false))
        );
        assert_eq!(
            evaluate("1 > 2 or 5 >= 3", &time, None),
            Some(device::Value::Bool(true))
        );

        assert_eq!(
            evaluate("{utc:second}", &time, None),
            Some(device::Value::Int(5))
        );
        assert_eq!(
            evaluate("{utc:minute}", &time, None),
            Some(device::Value::Int(4))
        );
        assert_eq!(
            evaluate("{utc:hour}", &time, None),
            Some(device::Value::Int(3))
        );
        assert_eq!(
            evaluate("{utc:day}", &time, None),
            Some(device::Value::Int(2))
        );
        assert_eq!(
            evaluate("{utc:month}", &time, None),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            evaluate("{utc:year}", &time, None),
            Some(device::Value::Int(2000))
        );
        assert_eq!(
            evaluate("{utc:DOW}", &time, None),
            Some(device::Value::Int(6))
        );
        assert_eq!(
            evaluate("{utc:DOY}", &time, None),
            Some(device::Value::Int(1))
        );
        assert_eq!(
            evaluate("{local:second}", &time, None),
            Some(device::Value::Int(10))
        );
        assert_eq!(
            evaluate("{local:minute}", &time, None),
            Some(device::Value::Int(9))
        );
        assert_eq!(
            evaluate("{local:hour}", &time, None),
            Some(device::Value::Int(8))
        );
        assert_eq!(
            evaluate("{local:day}", &time, None),
            Some(device::Value::Int(7))
        );
        assert_eq!(
            evaluate("{local:month}", &time, None),
            Some(device::Value::Int(6))
        );
        assert_eq!(
            evaluate("{local:year}", &time, None),
            Some(device::Value::Int(2001))
        );
        assert_eq!(
            evaluate("{local:DOW}", &time, None),
            Some(device::Value::Int(3))
        );
        assert_eq!(
            evaluate("{local:DOY}", &time, None),
            Some(device::Value::Int(157))
        );

        // Verify the solar variable are working correctly.

        assert_eq!(evaluate("{solar:alt}", &time, None), None);
        assert_eq!(evaluate("{solar:az}", &time, None), None);
        assert_eq!(evaluate("{solar:ra}", &time, None), None);
        assert_eq!(evaluate("{solar:dec}", &time, None), None);
        assert_eq!(
            evaluate("{solar:alt}", &time, Some(&solar)),
            Some(device::Value::Flt(1.0))
        );
        assert_eq!(
            evaluate("{solar:az}", &time, Some(&solar)),
            Some(device::Value::Flt(2.0))
        );
        assert_eq!(
            evaluate("{solar:ra}", &time, Some(&solar)),
            Some(device::Value::Flt(3.0))
        );
        assert_eq!(
            evaluate("{solar:dec}", &time, Some(&solar)),
            Some(device::Value::Flt(4.0))
        );

        const LY_TESTS: &[(i32, bool)] = &[
            (1964, true),
            (1996, true),
            (1997, false),
            (1998, false),
            (1999, false),
            (2000, true),
            (2001, false),
            (2002, false),
            (2003, false),
            (2004, true),
            (2096, true),
            (2097, false),
            (2098, false),
            (2099, false),
            (2100, false),
            (2101, false),
            (2102, false),
            (2103, false),
            (2104, true),
        ];

        for (year, is_ly) in LY_TESTS {
            let time = Arc::new((
                chrono::Utc
                    .with_ymd_and_hms(*year, 1, 2, 3, 4, 5)
                    .single()
                    .unwrap(),
                chrono::Local
                    .with_ymd_and_hms(*year, 6, 7, 8, 9, 10)
                    .single()
                    .unwrap(),
            ));

            assert_eq!(
                evaluate("{utc:LY}", &time, None),
                Some(device::Value::Bool(*is_ly)),
                "failed on UTC year {}",
                year
            );
            assert_eq!(
                evaluate("{local:LY}", &time, None),
                Some(device::Value::Bool(*is_ly)),
                "failed on local year {}",
                year
            );
        }
    }

    #[test]
    fn test_time_usage() {
        const DATA: &[(&str, Option<tod::TimeField>)] = &[
            // Make sure literals, variables, and solar values don't
            // return a field.
            ("{a}", None),
            ("1", None),
            ("1.0", None),
            ("true", None),
            ("#green", None),
            ("\"test\"", None),
            ("{solar:alt}", None),
            // Make sure the time values return the proper field.
            ("{utc:second}", Some(tod::TimeField::Second)),
            ("{utc:minute}", Some(tod::TimeField::Minute)),
            ("{utc:hour}", Some(tod::TimeField::Hour)),
            ("{utc:day}", Some(tod::TimeField::Day)),
            ("{utc:DOW}", Some(tod::TimeField::Day)),
            ("{utc:DOY}", Some(tod::TimeField::Day)),
            ("{utc:month}", Some(tod::TimeField::Month)),
            ("{utc:year}", Some(tod::TimeField::Year)),
            ("{utc:LY}", Some(tod::TimeField::Year)),
            ("{local:second}", Some(tod::TimeField::Second)),
            ("{local:minute}", Some(tod::TimeField::Minute)),
            ("{local:hour}", Some(tod::TimeField::Hour)),
            ("{local:day}", Some(tod::TimeField::Day)),
            ("{local:DOW}", Some(tod::TimeField::Day)),
            ("{local:DOY}", Some(tod::TimeField::Day)),
            ("{local:month}", Some(tod::TimeField::Month)),
            ("{local:year}", Some(tod::TimeField::Year)),
            ("{local:LY}", Some(tod::TimeField::Year)),
            // Now test more complicated expressions to make sure each
            // subtree is correctly compared.
            ("not (2 > 3)", None),
            ("2 + 2", None),
            ("{utc:second} + 2", Some(tod::TimeField::Second)),
            ("2 + {utc:second}", Some(tod::TimeField::Second)),
            ("{local:hour} + {utc:minute}", Some(tod::TimeField::Minute)),
            ("{local:minute} + {utc:day}", Some(tod::TimeField::Minute)),
        ];

        for (expr, result) in DATA {
            assert_eq!(
                &to_expr(expr).uses_time(),
                result,
                "error using {}",
                expr
            );
        }
    }

    #[test]
    fn test_solar_usage() {
        const DATA: &[(&str, bool)] = &[
            // Make sure literals, variables, and time values don't
            // return a field.
            ("{a}", false),
            ("1", false),
            ("1.0", false),
            ("true", false),
            ("#green", false),
            ("\"test\"", false),
            ("{utc:second}", false),
            // Make sure the solar values return true.
            ("{solar:alt}", true),
            ("{solar:dec}", true),
            ("{solar:ra}", true),
            ("{solar:az}", true),
            // Now test more complicated expressions to make sure each
            // subtree is correctly compared.
            ("not (2 > 3)", false),
            ("2 + 2", false),
            ("{solar:alt} + 2", true),
            ("2 + {solar:az}", true),
            ("{solar:dec} + {solar:az}", true),
        ];

        for (expr, result) in DATA {
            assert_eq!(
                &to_expr(expr).uses_solar(),
                result,
                "error using {}",
                expr
            );
        }
    }
}
