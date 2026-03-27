use vibelang::ast::AstNode;
use vibelang::lexer::Lexer;
use vibelang::parser::Parser;
use vibelang::semantic::collect_unused_warnings;

fn parse_source_with_file(path: &'static str, src: &str) -> AstNode {
    let mut lexer = Lexer::new_with_file(src, path);
    let tokens = lexer.tokenize().expect("lex");
    let mut parser = Parser::new(tokens);
    parser.parse().expect("parse")
}

#[test]
fn warnings_example_reports_unused_identifier_categories() {
    let path = "examples/misc/warnings.vc";
    let src = std::fs::read_to_string(path).expect("read warnings example");
    let ast = parse_source_with_file(path, &src);
    let warnings = collect_unused_warnings(&ast);
    let messages = warnings.iter().map(|w| w.message.as_str()).collect::<Vec<_>>();

    assert!(
        messages.iter().any(|m| *m == "unused import `Task`"),
        "missing unused import warning: {messages:?}"
    );
    assert!(
        messages.iter().any(|m| *m == "unused struct `S`"),
        "missing unused struct warning: {messages:?}"
    );
    assert!(
        messages.iter().any(|m| *m == "unused function `foo`"),
        "missing unused function warning: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| *m == "unused generic type parameter `T`"),
        "missing unused generic type parameter warning: {messages:?}"
    );
    assert!(
        messages.iter().any(|m| *m == "unused binding `b`"),
        "missing unused local binding warning: {messages:?}"
    );
}
