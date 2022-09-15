mod lexer;
use std::fs;
fn main() {
    let mut lexer = lexer::Lexer::new();
    let example_code = fs::read_to_string("../example-code2.asm").expect("Unable to read file");
    lexer.parse(example_code);

    println!("Hello m68k!");
}
