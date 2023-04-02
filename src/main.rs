use console::Term;
use s68k::{
    instructions::{Interrupt, InterruptResult},
    interpreter::{InterpreterStatus, InterpreterOptions, Interpreter},
    compiler::{InstructionLine},
    S68k,
};
use core::panic;
use std::fs;
use std::env;
use std::time::Instant;
enum StepKind {
    Step, 
    Undo, 
    Print, 
    Stop
}

fn main() {
    let example_code = fs::read_to_string("code-to-run.asm").expect("Unable to read file");
    let s68k = S68k::new(example_code);
    let args = env::args().collect::<Vec<String>>();
    if args.contains(&"--lex".to_string()){
        println!("\n---------LEXED---------\n");
        for line in s68k.get_lexed_lines() {
            println!("{:#?}", line);
        }
    }
    let errors = s68k.semantic_check();
    if !args.contains(&"--no-errors".to_string()){
        if errors.len() > 0 {
           println!("\n---------ERRORS--------\n");
           for error in errors.iter() {
                println!("{}", error.get_message());
            }
        }
    }
    if errors.len() > 0 {
        println!("\n");
        panic!("Errors found, aborting");
    }
    println!("\n----COMPILED-PROGRAM----\n");
    let compiled_program = s68k.compile().unwrap();
    //pre_interpreter.debug_print();
    if args.contains(&"--show-compiled".to_string()) {
        let mut instructions:Vec<InstructionLine> = compiled_program.get_instructions_map().into_values().collect();
        instructions.sort_by_key(|i| i.address);
        println!("{:#?}", instructions);
    }
    let options = InterpreterOptions{
        keep_history: true,
        ..Default::default()
    };
    //16 mb of memory
    let mut interpreter = s68k.create_interpreter(compiled_program, 0xFFFFFF, Some(options));
    //ask user if it wants to run the code (0) or allow to step through it (1)
    let execution_mode = if args.contains(&"--step".to_string()){
        "1".to_string()
    } else if args.contains(&"--run".to_string()){
        "0".to_string()
    } else {
        println!("Do you want to run the code (0) or step through it (1)?");
        let mut value = Term::stdout().read_line().expect("Unable to read line");
        while value != "0" && value != "1" {
            println!("Please enter 0 or 1");
            value = Term::stdout().read_line().expect("Unable to read line");
        }
        value
    };
    let start = Instant::now();
    if execution_mode == "0" {
        while !interpreter.has_terminated() {
            let status = interpreter.run().unwrap();
            match status {
                InterpreterStatus::Interrupt => {
                    let interrupt = interpreter.get_current_interrupt().unwrap();
                    handle_interrupt(&mut interpreter, &interrupt);
                }
                InterpreterStatus::TerminatedWithException => {
                    println!("Program Terminated with exception");
                }
                _ => {}
            }
        }
    } else {
        println!("D for step, A for undo, S for print, Q for quit");
        while !interpreter.has_terminated() {
            let step_kind = ask_step_kind();
            match step_kind {
                StepKind::Step => {
                    interpreter.step().unwrap();
                    let ins = interpreter.get_next_instruction();
                    println!("{:?}", ins);
                }
                StepKind::Undo => {
                    interpreter.undo().unwrap();
                }
                StepKind::Print => {
                    interpreter.debug_status();
                }
                StepKind::Stop => {
                    break;
                }
            }
            let status = interpreter.get_status();
            match status {
                InterpreterStatus::Interrupt => {
                    let interrupt = interpreter.get_current_interrupt().unwrap();
                    handle_interrupt(&mut interpreter, &interrupt);
                }
                InterpreterStatus::TerminatedWithException => {
                    println!("Program Terminated with exception");
                }
                _ => {}
            }
        }
    }
    if !args.contains(&"--no-debug".to_string()){
        interpreter.debug_status();
        println!("\nExecution took: {:?}", start.elapsed());
    }
}


fn ask_step_kind() -> StepKind {
    //D for next, A for previous, S for print, Q for quit   
    let mut step_kind = Term::stdout().read_line().expect("Unable to read line").to_uppercase();
    while step_kind != "D" && step_kind != "A" && step_kind != "S" && step_kind != "Q" {
        println!("Please enter D, A, S or Q");
        step_kind = Term::stdout().read_line().expect("Unable to read line");
    }
    match step_kind.as_str() {
        "D" => StepKind::Step,
        "A" => StepKind::Undo,
        "S" => StepKind::Print,
        "Q" => StepKind::Stop,
        _ => panic!("Invalid step kind")
    }
}

fn handle_interrupt(interpreter: &mut Interpreter, interrupt: &Interrupt) {
    match interrupt {
        Interrupt::DisplayNumber(number) => {
            print!("{}", number);
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
        Interrupt::DisplayChar(char) => {
            print!("{}", char);
            interpreter
                .answer_interrupt(InterruptResult::DisplayChar)
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