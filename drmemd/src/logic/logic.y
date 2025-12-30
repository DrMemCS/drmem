%expect-unused Unknown "UNKNOWN"

%start Logic
%parse-param p: &(&[String], &[String])

%avoid_insert "INT"
%avoid_insert "FLT"
%avoid_insert "IDENTIFIER"
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
%epp KW_IF "if"
%epp KW_THEN "then"
%epp KW_ELSE "else"
%epp ADD "+"
%epp SUB "-"
%epp MUL "*"
%epp DIV "/"
%epp REM "%"
%epp COLON ":"
%epp LBRACE "{"
%epp RBRACE "}"

%left KW_IF
%left KW_THEN
%left KW_ELSE

%%

Logic -> Result<Program>:
    TopExpr "CONTROL" "LBRACE" "IDENTIFIER" "RBRACE"
    {
	let v = $4.map_err(|_| Error::ParseError(
	        String::from("error reading target device")
            ))?;
	let s = $lexer.span_str(v.span());

	Ok(Program($1?, parse_device(s, p.1)?))
    }
    ;

TopExpr -> Result<Expr>:
      BoolExpr { $1 }
    ;

BoolExpr -> Result<Expr>:
    OrExpr { $1 }
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
      NumExpr "EQ" NumExpr { Ok(Expr::Eq(
			        Box::new($1?),
			        Box::new($3?)
			     )) }

    | NumExpr "NE" NumExpr { Ok(Expr::Not(
		                Box::new(
			           Expr::Eq(
			              Box::new($1?),
			              Box::new($3?)
			           )
			        )
		             )) }

    | NumExpr "GT" NumExpr { Ok(Expr::Lt(
			        Box::new($3?),
			        Box::new($1?)
			     )) }

    | NumExpr "GT_EQ" NumExpr { Ok(Expr::LtEq(
			           Box::new($3?),
			           Box::new($1?)
			        )) }

    | NumExpr "LT" NumExpr { Ok(Expr::Lt(
			        Box::new($1?),
			        Box::new($3?)
			     )) }

    | NumExpr "LT_EQ" NumExpr { Ok(Expr::LtEq(
			           Box::new($1?),
			           Box::new($3?)
			        )) }

    | NumExpr { $1 }
    ;

NumExpr -> Result<Expr>:
    AddSubExpr { $1 }
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
    | "(" TopExpr ")" { $2 }
    | Conditional { $1 }
    | "TRUE" { Ok(Expr::Lit(device::Value::Bool(true))) }
    | "FALSE" { Ok(Expr::Lit(device::Value::Bool(false))) }
    | "INT"
      {
	  let s = get_str("literal integer", $1, $lexer)?;

          parse_int(s)
      }
    | "FLT"
      {
	  let s = get_str("literal floating point", $1, $lexer)?;

	  parse_flt(s)
      }
    | "STRING"
    {
	let s = get_str("literal string", $1, $lexer)?;

	Ok(Expr::Lit(device::Value::Str(s[1..s.len() - 1].into())))
    }
    | "COLOR"
    {
	let s = get_str("literal color", $1, $lexer)?;

	match LinSrgba::<u8>::from_str(s) {
	    Ok(v) => Ok(Expr::Lit(device::Value::Color(v))),
	    Err(_) =>
		match LinSrgb::<u8>::from_str(s) {
		    Ok(v) =>
		        Ok(Expr::Lit(device::Value::Color(v.with_alpha(255u8)))),
		    Err(_) =>
		        match named::from_str(s) {
		            Some(v) => Ok(Expr::Lit(device::Value::Color(
		                Srgb::<f32>::from_format(v)
			            .into_linear()
			            .into_format::<u8>()
			            .with_alpha(255u8)
		            ))),
		            None => Err(Error::ParseError(
			        format!("invalid color '{s}'")
		            ))
		        }
	        }
	}
    }
    | Device { $1 }
    ;

Conditional -> Result<Expr>:
    "KW_IF" TopExpr "KW_THEN" TopExpr "KW_ELSE" TopExpr "KW_END"
      {
          Ok(Expr::If(Box::new($2?), Box::new($4?), Some(Box::new($6?))))
      }
    | "KW_IF" TopExpr "KW_THEN" TopExpr "KW_END"
      {
          Ok(Expr::If(Box::new($2?), Box::new($4?), None))
      }
    ;

Device -> Result<Expr>:
    "LBRACE" "IDENTIFIER" "COLON" "IDENTIFIER" "RBRACE"
    {
	let lexer = $lexer;
	let cat = get_str("built-in category", $2, lexer)?;
	let fld = get_str("built-in field", $4, lexer)?;

	parse_builtin(cat, fld)
    }
    | "LBRACE" "IDENTIFIER" "RBRACE"
    {
	let s = get_str("device name", $2, $lexer)?;

	Ok(Expr::Var(parse_device(s, p.0)?))
    }
    ;

Unknown -> ():
    "UNKNOWN" { }
    ;

%%

use drmem_api::{Result, Error, device};
use palette::{LinSrgba, LinSrgb, Srgb, named, WithAlpha};
use super::{Zone, TimeField, SolarField, Expr, Program};
use std::str::FromStr;

use lrlex::{DefaultLexeme, DefaultLexerTypes};
use lrpar::NonStreamingLexer;

// This complicated beast is an attempt to remove the boilerplate code
// used when processing the terminal tokens (i.e. the leaf values of
// the expression tree.)

fn get_str<'a, 'input>(
    label: &'a str,
    lexeme: std::result::Result<DefaultLexeme, DefaultLexeme>,
    lexer: &'a (dyn NonStreamingLexer<'input, DefaultLexerTypes> + 'a)
) -> Result<&'input str> {
    let lexeme = lexeme.map_err(|_| Error::ParseError(
        format!("error reading {label}")
    ))?;

    Ok(lexer.span_str(lexeme.span()))
}

// Any functions here are in scope for all the grammar actions above.

fn parse_int(s: &str) -> Result<Expr> {
    s.parse::<i32>()
	.map(|v| Expr::Lit(device::Value::Int(v)))
	.map_err(|_| Error::ParseError(
	     format!("{s} cannot be represented as an i32")
	))
}

fn parse_flt(s: &str) -> Result<Expr> {
    s.parse::<f64>()
	.map(|v| Expr::Lit(device::Value::Flt(v)))
	.map_err(|_| Error::ParseError(
	     format!("{s} cannot be represented as an f64")
	))
}

fn parse_device(name: &str, env: &[String]) -> Result<usize> {
    for ii in env.iter().enumerate() {
        if *ii.1 == name {
	    return Ok(ii.0);
	}
    }
    Err(Error::ParseError(format!("variable '{}' is not defined", &name)))
}

const CAT_UTC: &str = "utc";
const CAT_LOCAL: &str = "local";
const CAT_SOLAR: &str = "solar";

const FLD_SECOND: &str = "second";
const FLD_MINUTE: &str = "minute";
const FLD_HOUR: &str = "hour";
const FLD_DAY: &str = "day";
const FLD_MONTH: &str = "month";
const FLD_YEAR: &str = "year";
const FLD_DOW: &str = "DOW";
const FLD_DOY: &str = "DOY";
const FLD_EOM: &str = "EOM";
const FLD_SOM: &str = "SOM";
const FLD_LY: &str = "LY";
const FLD_ALT: &str = "alt";
const FLD_AZ: &str = "az";
const FLD_RA: &str = "ra";
const FLD_DEC: &str = "dec";

fn parse_builtin(cat: &str, fld: &str) -> Result<Expr> {
    match (cat, fld) {
	(CAT_UTC, FLD_SECOND) => Ok(Expr::TimeVal(
            Zone::Utc, TimeField::Second
        )),
	(CAT_UTC, FLD_MINUTE) => Ok(Expr::TimeVal(
            Zone::Utc, TimeField::Minute
        )),
	(CAT_UTC, FLD_HOUR) => Ok(Expr::TimeVal(
	    Zone::Utc, TimeField::Hour
        )),
	(CAT_UTC, FLD_DAY) => Ok(Expr::TimeVal(
	    Zone::Utc, TimeField::Day
        )),
	(CAT_UTC, FLD_DOW) => Ok(Expr::TimeVal(
            Zone::Utc, TimeField::DoW
        )),
	(CAT_UTC, FLD_MONTH) => Ok(Expr::TimeVal(
            Zone::Utc, TimeField::Month
        )),
	(CAT_UTC, FLD_SOM) => Ok(Expr::TimeVal(
            Zone::Utc, TimeField::SoM
        )),
	(CAT_UTC, FLD_EOM) => Ok(Expr::TimeVal(
            Zone::Utc, TimeField::EoM
        )),
	(CAT_UTC, FLD_YEAR) => Ok(Expr::TimeVal(
            Zone::Utc, TimeField::Year
        )),
	(CAT_UTC, FLD_LY) => Ok(Expr::TimeVal(
            Zone::Utc, TimeField::LeapYear
        )),
	(CAT_UTC, FLD_DOY) => Ok(Expr::TimeVal(
            Zone::Utc, TimeField::DoY
        )),
	(CAT_LOCAL, FLD_SECOND) => Ok(Expr::TimeVal(
            Zone::Local, TimeField::Second
        )),
	(CAT_LOCAL, FLD_MINUTE) => Ok(Expr::TimeVal(
            Zone::Local, TimeField::Minute
        )),
	(CAT_LOCAL, FLD_HOUR) => Ok(Expr::TimeVal(
            Zone::Local, TimeField::Hour
        )),
	(CAT_LOCAL, FLD_DAY) => Ok(Expr::TimeVal(
            Zone::Local, TimeField::Day
        )),
	(CAT_LOCAL, FLD_DOW) => Ok(Expr::TimeVal(
            Zone::Local, TimeField::DoW
        )),
	(CAT_LOCAL, FLD_MONTH) => Ok(Expr::TimeVal(
            Zone::Local, TimeField::Month
        )),
	(CAT_LOCAL, FLD_SOM) => Ok(Expr::TimeVal(
            Zone::Local, TimeField::SoM
        )),
	(CAT_LOCAL, FLD_EOM) => Ok(Expr::TimeVal(
            Zone::Local, TimeField::EoM
        )),
	(CAT_LOCAL, FLD_YEAR) => Ok(Expr::TimeVal(
            Zone::Local, TimeField::Year
        )),
	(CAT_LOCAL, FLD_LY) => Ok(Expr::TimeVal(
            Zone::Local, TimeField::LeapYear
        )),
	(CAT_LOCAL, FLD_DOY) => Ok(Expr::TimeVal(
            Zone::Local, TimeField::DoY
        )),
	(CAT_SOLAR, FLD_ALT) => Ok(Expr::SolarVal(SolarField::Elevation)),
	(CAT_SOLAR, FLD_AZ) => Ok(Expr::SolarVal(SolarField::Azimuth)),
	(CAT_SOLAR, FLD_RA) => Ok(Expr::SolarVal(SolarField::RightAscension)),
	(CAT_SOLAR, FLD_DEC) => Ok(Expr::SolarVal(SolarField::Declination)),
	_ => Err(Error::ParseError(
		 format!("unknown built-in: {cat}:{fld}")
	     ))
    }
}
