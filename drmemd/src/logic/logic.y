%start Logic
%avoid_insert "INT"
%avoid_insert "FLT"

%%

Logic -> Result<Program, ()>:
    Expr "CONTROL" "DEVICE"
    {
	let v = $3.map_err(|_| ())?;
	let s = $lexer.span_str(v.span());

	Ok(Program::Assign($1?, parse_device(s)))
    }
    ;

Expr -> Result<Expr, ()>:
    Factor { $1 }
    ;

Factor -> Result<Expr, ()>:
    '(' Expr ')' { $2 }
    | "INT"
      {
          let v = $1.map_err(|_| ())?;

          parse_int($lexer.span_str(v.span()))
      }
    | "FLT"
      {
          let v = $1.map_err(|_| ())?;

	  parse_flt($lexer.span_str(v.span()))
      }
    | Device { $1 }
    ;

Device -> Result<Expr, ()>:
    "DEVICE"
    {
	let v = $1.map_err(|_| ())?;
	let s = $lexer.span_str(v.span());

	Ok(Expr::Var(parse_device(s)))
    }
    ;

%%

use drmem_api::types::device::Value;
use super::{Expr, Program};

// Any functions here are in scope for all the grammar actions above.

fn parse_int(s: &str) -> Result<Expr, ()> {
    s.parse::<i32>()
	.map(|v| Expr::Lit(Value::Int(v)))
	.map_err(|_| eprintln!("{} cannot be represented as an i32", s))
}

fn parse_flt(s: &str) -> Result<Expr, ()> {
    s.parse::<f64>()
	.map(|v| Expr::Lit(Value::Flt(v)))
	.map_err(|_| eprintln!("{} cannot be represented as an f64", s))
}

fn parse_device(s: &str) -> String {
    s[1..s.len() - 1].to_string()
}
