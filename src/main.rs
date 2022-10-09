use console::Term;
use s68k::{
    instructions::{Interrupt, InterruptResult},
    interpreter::InterpreterStatus,
    S68k,
};
use std::fs;
fn main() {
    let example_code = fs::read_to_string("code-to-run.asm").expect("Unable to read file");
    let s68k = S68k::new(example_code);
    println!("\n---------LEXED---------\n");
    for line in s68k.get_lexed_lines() {
        //println!("{:#?}", line);
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
    //pre_interpreter.debug_print();

    //16 mb of memory
    let mut interpreter = s68k.create_interpreter(pre_interpreter, 0xFFFFFF);

    while !interpreter.has_terminated() {
        let status = interpreter.run().unwrap();
        println!("Interpreter paused with status: {:?}", status);
        match status {
            InterpreterStatus::Interrupt => {
                let interrupt = interpreter.get_current_interrupt().unwrap();
                match interrupt {
                    Interrupt::DisplayNumber(number) => {
                        println!("{}", number);
                        interpreter
                            .answer_interrupt(InterruptResult::DisplayNumber)
                            .unwrap();
                    }
                    Interrupt::DisplayStringWithCRLF(string) => {
                        println!("{}", string);
                        interpreter
                            .answer_interrupt(InterruptResult::DisplayStringWithCRLF)
                            .unwrap();
                    }
                    Interrupt::DisplayStringWithoutCRLF(string) => {
                        print!("{}", string);
                        interpreter
                            .answer_interrupt(InterruptResult::DisplayStringWithoutCRLF)
                            .unwrap();
                    }
                    Interrupt::GetTime => {
                        interpreter
                            .answer_interrupt(InterruptResult::GetTime(0))
                            .unwrap();
                    }
                    Interrupt::ReadChar => {
                        let char = Term::stdout().read_char().expect("Unable to read char");
                        interpreter
                            .answer_interrupt(InterruptResult::ReadChar(char))
                            .unwrap();
                    }
                    Interrupt::ReadNumber => {
                        let num = Term::stdout().read_line().expect("Unable to read line");
                        let num = num.trim().parse::<i32>().expect("Unable to parse number");
                        interpreter
                            .answer_interrupt(InterruptResult::ReadNumber(num))
                            .unwrap();
                    }
                    Interrupt::ReadKeyboardString => {
                        let string = Term::stdout().read_line().expect("Unable to read line");
                        interpreter
                            .answer_interrupt(InterruptResult::ReadKeyboardString(string))
                            .unwrap();
                    }
                    Interrupt::Terminate => {
                        interpreter
                            .answer_interrupt(InterruptResult::Terminate)
                            .unwrap();
                    }
                }
            }
            InterpreterStatus::TerminatedWithException => {
                println!("Program Terminated with exception");
            }
            _ => {}
        }
    }
    interpreter.debug_status();
}
