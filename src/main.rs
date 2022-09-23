
use std::fs;

use s68k::S68k;
fn main() {
    let example_code = fs::read_to_string("example-code2.asm").expect("Unable to read file");
    let s68k = S68k::new(example_code);
    for line in s68k.get_lexed_lines(){
        println!("{:?}", line);
    }
    for error in s68k.semantic_check() {
        println!("{}", error.get_message());
    }
}

