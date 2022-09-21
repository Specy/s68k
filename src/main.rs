mod lexer;
mod constants;
mod semantic_checker;

use semantic_checker::SemanticChecker;
use lexer::Lexer;
use std::fs;
fn main() {
    let mut lexer = Lexer::new();
    let example_code = fs::read_to_string("example-code2.asm").expect("Unable to read file");
    lexer.lex(example_code);
    for line in lexer.get_lines() {
        println!("{:?}", line.parsed);
    }
    let lines = lexer.get_lines();
    let semantic_checker = SemanticChecker::new(&lines);
    let errors = semantic_checker.get_errors();
    for error in errors {
        println!("{}", error.get_message());
    }
}

