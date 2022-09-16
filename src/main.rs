mod lexer;
mod constants;
use std::fs;
fn main() {
    let mut lexer = lexer::Lexer::new();
    let example_code = fs::read_to_string("../example-code2.asm").expect("Unable to read file");
    lexer.lex(example_code);

    for line in lexer.lines {
        println!("{:?}", line);
    }
    println!("Hello m68k!");
}
