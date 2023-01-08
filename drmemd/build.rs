use cfgrammar::yacc::YaccKind;
use lrlex::CTLexerBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    CTLexerBuilder::new()
        .lrpar_config(|ctp| {
            ctp.yacckind(YaccKind::Grmtools)
                .recoverer(lrpar::RecoveryKind::None)
                .grammar_in_src_dir("logic/logic.y")
                .unwrap()
        })
        .lexer_in_src_dir("logic/logic.l")?
        .build()?;
    Ok(())
}
