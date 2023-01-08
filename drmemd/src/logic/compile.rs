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

#[derive(Debug, PartialEq)]
pub enum Expr {
    Lit(Value),
    Var(String),
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
    }
}
