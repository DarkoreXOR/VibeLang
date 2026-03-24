//! Vibelang compiler front end (lexer, parser, AST visitors, VM runtime).

pub mod ast;
pub mod bytecode;
pub mod bytecode_gen;
pub mod builtins;
pub mod error;
pub mod lexer;
pub mod module_loader;
pub mod monomorphize;
pub mod parser;
pub mod semantic;
pub mod type_key;
pub mod vm;
pub mod value;
pub mod visit;
