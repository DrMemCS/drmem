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
%epp ADD "+"
%epp SUB "-"
%epp MUL "*"
%epp DIV "/"
%epp REM "%"
%epp COLON ":"
%epp LBRACE "{"
%epp RBRACE "}"

%%

Logic -> Result<Program>:
    OrExpr "CONTROL" "LBRACE" "IDENTIFIER" "RBRACE"
    {
	let v = $4.map_err(|_| Error::ParseError(
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
    | "COLOR"
    {
	let v = $1.map_err(|_| Error::ParseError(
	        String::from("error reading literal color")
            ))?;
	let s = $lexer.span_str(v.span());

	match LinSrgb::<u8>::from_str(s) {
	    Ok(v) => Ok(Expr::Lit(device::Value::Color(v))),
	    Err(_) =>
		match named::from_str(s) {
		    Some(v) => Ok(Expr::Lit(device::Value::Color(
		        Srgb::<f32>::from_format(v)
			    .into_linear()
			    .into_format()
		    ))),
		    None => Err(Error::ParseError(
			format!("invalid color '{}'", s)
		    ))
		}
	}
    }
    | Device { $1 }
    ;

Device -> Result<Expr>:
    "LBRACE" "IDENTIFIER" "COLON" "IDENTIFIER" "RBRACE"
    {
	let vcat = $2.map_err(|_| Error::ParseError(
	        String::from("error reading built-in category")
            ))?;
	let cat = $lexer.span_str(vcat.span());
	let vfld = $4.map_err(|_| Error::ParseError(
	        String::from("error reading built-in field")
            ))?;
	let fld = $lexer.span_str(vfld.span());

	parse_builtin(cat, fld)
    }
    | "LBRACE" "IDENTIFIER" "RBRACE"
    {
	let v = $2.map_err(|_| Error::ParseError(
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
use chrono::{Timelike, Datelike};
use palette::{LinSrgb, Srgb, named};
use super::{super::tod, super::solar, Expr, Program};
use std::str::FromStr;

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
const FLD_ALT: &str = "alt";
const FLD_AZ: &str = "az";
const FLD_RA: &str = "ra";
const FLD_DEC: &str = "dec";

fn get_utc_second(info: &tod::Info) -> device::Value {
    device::Value::Int(info.0.second() as i32)
}

fn get_utc_minute(info: &tod::Info) -> device::Value {
    device::Value::Int(info.0.minute() as i32)
}

fn get_utc_hour(info: &tod::Info) -> device::Value {
    device::Value::Int(info.0.hour() as i32)
}

fn get_utc_day(info: &tod::Info) -> device::Value {
    device::Value::Int(info.0.day() as i32)
}

fn get_utc_day_of_week(info: &tod::Info) -> device::Value {
    device::Value::Int(info.0.weekday().num_days_from_monday() as i32)
}

fn get_utc_month(info: &tod::Info) -> device::Value {
    device::Value::Int(info.0.month() as i32)
}

fn get_utc_year(info: &tod::Info) -> device::Value {
    device::Value::Int(info.0.year())
}

fn get_utc_day_of_year(info: &tod::Info) -> device::Value {
    device::Value::Int(info.0.ordinal0() as i32)
}

fn get_local_second(info: &tod::Info) -> device::Value {
    device::Value::Int(info.1.second() as i32)
}

fn get_local_minute(info: &tod::Info) -> device::Value {
    device::Value::Int(info.1.minute() as i32)
}

fn get_local_hour(info: &tod::Info) -> device::Value {
    device::Value::Int(info.1.hour() as i32)
}

fn get_local_day(info: &tod::Info) -> device::Value {
    device::Value::Int(info.1.day() as i32)
}

fn get_local_day_of_week(info: &tod::Info) -> device::Value {
    device::Value::Int(info.1.weekday().num_days_from_monday() as i32)
}

fn get_local_month(info: &tod::Info) -> device::Value {
    device::Value::Int(info.1.month() as i32)
}

fn get_local_year(info: &tod::Info) -> device::Value {
    device::Value::Int(info.1.year())
}

fn get_local_day_of_year(info: &tod::Info) -> device::Value {
    device::Value::Int(info.1.ordinal0() as i32)
}

fn get_solar_altitude(info: &solar::Info) -> device::Value {
    device::Value::Flt(info.elevation)
}

fn get_solar_azimuth(info: &solar::Info) -> device::Value {
    device::Value::Flt(info.azimuth)
}

fn get_solar_right_ascension(info: &solar::Info) -> device::Value {
    device::Value::Flt(info.right_ascension)
}

fn get_solar_declination(info: &solar::Info) -> device::Value {
    device::Value::Flt(info.declination)
}

fn parse_builtin(cat: &str, fld: &str) -> Result<Expr> {
    match (cat, fld) {
	(CAT_UTC, FLD_SECOND) => Ok(Expr::TimeVal(
            CAT_UTC, FLD_SECOND, get_utc_second
        )),
	(CAT_UTC, FLD_MINUTE) => Ok(Expr::TimeVal(
            CAT_UTC, FLD_MINUTE, get_utc_minute
        )),
	(CAT_UTC, FLD_HOUR) => Ok(Expr::TimeVal(
            CAT_UTC, FLD_HOUR, get_utc_hour
        )),
	(CAT_UTC, FLD_DAY) => Ok(Expr::TimeVal(
            CAT_UTC, FLD_DAY, get_utc_day
        )),
	(CAT_UTC, FLD_DOW) => Ok(Expr::TimeVal(
            CAT_UTC, FLD_DOW, get_utc_day_of_week
        )),
	(CAT_UTC, FLD_MONTH) => Ok(Expr::TimeVal(
            CAT_UTC, FLD_MONTH, get_utc_month
        )),
	(CAT_UTC, FLD_YEAR) => Ok(Expr::TimeVal(
            CAT_UTC, FLD_YEAR, get_utc_year
        )),
	(CAT_UTC, FLD_DOY) => Ok(Expr::TimeVal(
            CAT_UTC, FLD_DOY, get_utc_day_of_year
        )),
	(CAT_LOCAL, FLD_SECOND) => Ok(Expr::TimeVal(
            CAT_LOCAL, FLD_SECOND, get_local_second
        )),
	(CAT_LOCAL, FLD_MINUTE) => Ok(Expr::TimeVal(
            CAT_LOCAL, FLD_MINUTE, get_local_minute
        )),
	(CAT_LOCAL, FLD_HOUR) => Ok(Expr::TimeVal(
            CAT_LOCAL, FLD_HOUR, get_local_hour
        )),
	(CAT_LOCAL, FLD_DAY) => Ok(Expr::TimeVal(
            CAT_LOCAL, FLD_DAY, get_local_day
        )),
	(CAT_LOCAL, FLD_DOW) => Ok(Expr::TimeVal(
            CAT_LOCAL, FLD_DOW, get_local_day_of_week
        )),
	(CAT_LOCAL, FLD_MONTH) => Ok(Expr::TimeVal(
            CAT_LOCAL, FLD_MONTH, get_local_month
        )),
	(CAT_LOCAL, FLD_YEAR) => Ok(Expr::TimeVal(
            CAT_LOCAL, FLD_YEAR, get_local_year
        )),
	(CAT_LOCAL, FLD_DOY) => Ok(Expr::TimeVal(
            CAT_LOCAL, FLD_DOY, get_local_day_of_year
        )),
	(CAT_SOLAR, FLD_ALT) => Ok(Expr::SolarVal(
            FLD_ALT, get_solar_altitude
        )),
	(CAT_SOLAR, FLD_AZ) => Ok(Expr::SolarVal(
            FLD_AZ, get_solar_azimuth
        )),
	(CAT_SOLAR, FLD_RA) => Ok(Expr::SolarVal(
            FLD_RA, get_solar_right_ascension
        )),
	(CAT_SOLAR, FLD_DEC) => Ok(Expr::SolarVal(
            FLD_DEC, get_solar_declination
        )),
	_ => Err(Error::ParseError(
		 format!("unknown built-in: {}:{}", cat, fld)
	     ))
    }
}
