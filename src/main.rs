use s68k::S68k;
use std::fs;
fn main() {
    let example_code = fs::read_to_string("example-code2.asm").expect("Unable to read file");
    let s68k = S68k::new(example_code);
    println!("\n---------LEXED---------\n");
    for line in s68k.get_lexed_lines() {
        println!("{:#?}", line);
    }
    let errors = s68k.semantic_check();
    if errors.len() > 0 {
        println!("---------ERRORS--------");
    }
    for error in errors {
        println!("{}", error.get_message());
    }
    println!("\n----PRE-INTERPRETER----\n");
    let pre_interpreter = s68k.pre_process();
    pre_interpreter.debug_print();

    //16 mb of memory
    let mut interpreter = s68k.create_interpreter(pre_interpreter, 0xFFFFFF);
    interpreter.run();
}
