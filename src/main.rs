use std::env;
use std::fs;
use std::process;

use vibelang::lexer::Lexer;
use vibelang::parser::Parser;
use vibelang::semantic::{check_program, collect_unused_warnings};
use vibelang::error::{ParseError, Span};
use vibelang::bytecode_gen::compile_program;
use vibelang::module_loader::load_linked_program;

fn print_span_snippet(source: &str, span: Span) {
    // If the span came from an imported module, prefer that module's source for snippet printing.
    let module_source = span.file.and_then(|path| fs::read_to_string(path).ok());
    let source_for_snippet = match &module_source {
        Some(s) => s.as_str(),
        None => source,
    };

    let line_idx = span.line.saturating_sub(1);
    let line = source_for_snippet
        .lines()
        .nth(line_idx)
        .unwrap_or("<failed to fetch source line>");

    eprintln!("  {}", line);

    let col0 = span.column.saturating_sub(1);
    let caret_len = std::cmp::max(1, span.len);

    // Best-effort column alignment: `Span.column` is in characters, which usually matches terminal output.
    let caret_line = format!(
        "{}{}",
        " ".repeat(col0),
        "^".repeat(caret_len)
    );
    eprintln!("  {}", caret_line);
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "Usage: {} [--dump-tokens] [--dump-ast] <file.vc>",
            args.first().map(String::as_str).unwrap_or("vibelang")
        );
        process::exit(1);
    }

    let mut dump_tokens = false;
    let mut dump_ast = false;
    let mut paths = Vec::new();
    for a in args.iter().skip(1) {
        match a.as_str() {
            "--dump-tokens" => dump_tokens = true,
            "--dump-ast" => dump_ast = true,
            _ => paths.push(a.clone()),
        }
    }

    if paths.len() != 1 {
        eprintln!("Expected exactly one source file.");
        process::exit(1);
    }
    let filename = &paths[0];

    let source = match fs::read_to_string(filename) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: {}", filename, e);
            process::exit(1);
        }
    };

    let warning_ast = {
        let warning_file: &'static str = Box::leak(filename.clone().into_boxed_str());
        let mut warning_lexer = Lexer::new_with_file(&source, warning_file);
        let warning_tokens = match warning_lexer.tokenize() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{}", e.format_with_file(filename));
                print_span_snippet(&source, e.span);
                process::exit(1);
            }
        };
        let mut warning_parser = Parser::new(warning_tokens);
        match warning_parser.parse() {
            Ok(ast) => ast,
            Err(e) => {
                eprintln!("{}", e.format_with_file(filename));
                if let ParseError::UnexpectedToken { span: Some(s), .. } = &e {
                    print_span_snippet(&source, *s);
                }
                process::exit(1);
            }
        }
    };

    let ast = if dump_tokens || dump_ast {
        let mut lexer = Lexer::new(&source);
        let tokens = match lexer.tokenize() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{}", e.format_with_file(filename));
                print_span_snippet(&source, e.span);
                process::exit(1);
            }
        };

        if dump_tokens {
            println!("Tokens: {:#?}", tokens);
        }

        let mut parser = Parser::new(tokens);
        let ast = match parser.parse() {
            Ok(ast) => ast,
            Err(e) => {
                eprintln!("{}", e.format_with_file(filename));
                if let ParseError::UnexpectedToken { span: Some(s), .. } = &e {
                    print_span_snippet(&source, *s);
                }
                process::exit(1);
            }
        };

        if dump_ast {
            println!("AST: {:#?}", ast);
        }

        ast
    } else {
        match load_linked_program(filename) {
            Ok(ast) => ast,
            Err(e) => {
                if let Some(path) = e.path {
                    eprintln!("{}: {}", path.display(), e.message);
                } else {
                    eprintln!("{e:?}");
                }
                process::exit(1);
            }
        }
    };

    let sem_errors = check_program(&ast);
    let sem_warnings = collect_unused_warnings(&warning_ast);
    for warning in &sem_warnings {
        let header_path: &str = match warning.span.file {
            Some(p) => p,
            None => filename.as_str(),
        };
        eprintln!("{}", warning.format_with_file(header_path));
        print_span_snippet(&source, warning.span);
        eprintln!();
    }

    if !sem_errors.is_empty() {
        for err in &sem_errors {
            let header_path: &str = match err.span.file {
                Some(p) => p,
                None => filename.as_str(),
            };
            eprintln!("{}", err.format_with_file(header_path));
            print_span_snippet(&source, err.span);
            eprintln!();
        }
        process::exit(1);
    }

    let bytecode = match compile_program(&ast) {
        Ok(bc) => bc,
        Err(e) => {
            eprintln!("{}", e.format_with_file(filename));
            process::exit(1);
        }
    };
    if let Err(e) = vibelang::vm::run_program(&bytecode) {
        eprintln!("{}", e.format_with_file(filename));
        if let Some(s) = e.span {
            print_span_snippet(&source, s);
        }
        process::exit(1);
    }
}
