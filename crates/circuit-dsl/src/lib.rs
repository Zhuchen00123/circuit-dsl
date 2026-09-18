//! Lexer, parser, and elaboration for the circuit DSL.
//!
//! Pipeline position:
//!
//! ```text
//! source text
//!   -> lexer::lex      -> Vec<Token>
//!   -> parser::parse   -> ast::Program
//!   -> elaborate       -> (Circuit, AnalysisPlan)
//! ```
//!
//! Everything here is pure: no file or network access, and no evaluation of
//! user code during simulation. The DSL decides the circuit; the backend only
//! solves it.

pub mod ast;
pub mod elaborate;
pub mod lexer;
pub mod parser;
pub mod token;

pub use ast::Program;
pub use elaborate::{Compiled, Elaborated, compile, elaborate_experiment};
pub use lexer::lex;
pub use parser::parse;
