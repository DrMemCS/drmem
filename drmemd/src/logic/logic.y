%expect-unused Unknown "UNKNOWN"

%start Logic
%parse-param p: &(&[String], &[String])

%avoid_insert "INT"
%avoid_insert "FLT"
%avoid_insert "DEVICE"
%avoid_insert "TRUE"
%avoid_insert "FALSE"

%epp EQ "="
%epp NE "<>"
%epp GT ">"
%epp GT_EQ ">="
%epp LT "<"
%epp LT_EQ "<="
%epp B_NOT "not"
%epp B_AND "and"
%epp B_OR "or"
%epp ADD "+"
%epp SUB "-"
%epp MUL "*"
%epp DIV "/"
%epp REM "%"

%%

Logic -> Result<Program>:
    OrExpr "CONTROL" "DEVICE"
    {
	let v = $3.map_err(|_| Error::ParseError(
	        String::from("error reading target device")
            ))?;
	let s = $lexer.span_str(v.span());

	Ok(Program($1?, parse_device(s, p.1)?))
    }
    ;

OrExpr -> Result<Expr>:
      AndExpr "B_OR" OrExpr { Ok(Expr::Or(
			    Box::new($1?),
			    Box::new($3?)
			  )) }
    | AndExpr { $1 }
    ;

AndExpr -> Result<Expr>:
      CmpExpr "B_AND" AndExpr { Ok(Expr::And(
			    Box::new($1?),
			    Box::new($3?)
			  )) }
    | CmpExpr { $1 }
    ;

CmpExpr -> Result<Expr>:
      AddSubExpr "EQ" AddSubExpr { Ok(Expr::Eq(
			    Box::new($1?),
			    Box::new($3?)
			  )) }

    | AddSubExpr "NE" AddSubExpr { Ok(Expr::Not(
		            Box::new(
			      Expr::Eq(
			        Box::new($1?),
			        Box::new($3?)
			      )
			    )
		          )) }

    | AddSubExpr "GT" AddSubExpr { Ok(Expr::Lt(
			        Box::new($3?),
			        Box::new($1?)
			      )) }

    | AddSubExpr "GT_EQ" AddSubExpr { Ok(Expr::LtEq(
			        Box::new($3?),
			        Box::new($1?)
			      )) }

    | AddSubExpr "LT" AddSubExpr { Ok(Expr::Lt(
			    Box::new($1?),
			    Box::new($3?)
			  )) }

    | AddSubExpr "LT_EQ" AddSubExpr { Ok(Expr::LtEq(
			       Box::new($1?),
			       Box::new($3?)
			     )) }

    | AddSubExpr { $1 }
    ;

AddSubExpr -> Result<Expr>:
      MulDivExpr "ADD" AddSubExpr { Ok(Expr::Add(Box::new($1?), Box::new($3?))) }
    | MulDivExpr "SUB" AddSubExpr { Ok(Expr::Sub(Box::new($1?), Box::new($3?))) }
    | MulDivExpr { $1 }
    ;

MulDivExpr -> Result<Expr>:
      Expr "MUL" MulDivExpr { Ok(Expr::Mul(Box::new($1?), Box::new($3?))) }
    | Expr "DIV" MulDivExpr { Ok(Expr::Div(Box::new($1?), Box::new($3?))) }
    | Expr "REM" MulDivExpr { Ok(Expr::Rem(Box::new($1?), Box::new($3?))) }
    | Expr { $1 }
    ;

Expr -> Result<Expr>:
    Factor { $1 }
    ;

Factor -> Result<Expr>:
      "B_NOT" Factor { Ok(Expr::Not(Box::new($2?))) }
    | "(" OrExpr ")" { $2 }
    | "TRUE" { Ok(Expr::Lit(device::Value::Bool(true))) }
    | "FALSE" { Ok(Expr::Lit(device::Value::Bool(false))) }
    | "INT"
      {
          let v = $1.map_err(|_| Error::ParseError(
	        String::from("error reading literal integer value")
            ))?;

          parse_int($lexer.span_str(v.span()))
      }
    | "FLT"
      {
          let v = $1.map_err(|_| Error::ParseError(
	        String::from("error reading literal floating point")
            ))?;

	  parse_flt($lexer.span_str(v.span()))
      }
    | "STRING"
    {
	let v = $1.map_err(|_| Error::ParseError(
	        String::from("error reading literal string")
            ))?;
	let s = $lexer.span_str(v.span());

	Ok(Expr::Lit(device::Value::Str(s[1..s.len() - 1].to_string())))
    }
    | Device { $1 }
    ;

Device -> Result<Expr>:
    "DEVICE"
    {
	let v = $1.map_err(|_| Error::ParseError(
	        String::from("error reading device name")
            ))?;
	let s = $lexer.span_str(v.span());

	Ok(Expr::Var(parse_device(s, p.0)?))
    }
    ;

Unknown -> ():
    "UNKNOWN" { }
    ;

%%

use drmem_api::{Result, Error, device};
use super::{Expr, Program};

// Any functions here are in scope for all the grammar actions above.

fn parse_int(s: &str) -> Result<Expr> {
    s.parse::<i32>()
	.map(|v| Expr::Lit(device::Value::Int(v)))
	.map_err(|_| Error::ParseError(
	     format!("{} cannot be represented as an i32", s)
	))
}

fn parse_flt(s: &str) -> Result<Expr> {
    s.parse::<f64>()
	.map(|v| Expr::Lit(device::Value::Flt(v)))
	.map_err(|_| Error::ParseError(
	     format!("{} cannot be represented as an f64", s)
	))
}

fn parse_device(s: &str, env: &[String]) -> Result<usize> {
    let name = s[1..s.len() - 1].to_string();

    for ii in env.iter().enumerate() {
        if *ii.1 == name {
	    return Ok(ii.0);
	}
    }
    Err(Error::ParseError(format!("variable '{}' is not defined", &name)))
}
