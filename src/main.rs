//! The command line front end: assemble one File and run it.
//!
//! It is the quickest way to see what the Assembler and the Interpreter make
//! of a program without the editor: `cargo run -- program.asm` assembles it,
//! prints every Diagnostic as `file:line:column: severity: message` and, when
//! none of them is an error, runs it.
//!
//! ```text
//! Usage: s68k [FILE] [OPTIONS]
//!
//!   FILE              the Entry file to assemble (default code-to-run.asm)
//!   --step            step through the program instead of running it
//!   --benchmark       run it with no history kept and time it
//!   --run             run it, which is what it does anyway
//!   --show-program    print the assembled instructions before running
//!   --no-debug        do not print the registers when the run ends
//! ```
//!
//! It never asks a question: with no terminal to answer it — a pipe, a CI job,
//! an empty standard input — a prompt is an endless loop, and the mode is a
//! flag instead. In `--step` mode the keys are read one line at a time and
//! anything that is not one of them — the end of the input included — stops the
//! run rather than repeating the question.

use console::Term;
use s68k::assembler::assemble;
use s68k::assembler::diagnostics::Diagnostic;
use s68k::assembler::program::Program;
use s68k::assembler::source::{normalise_path, Files};
use s68k::{
    instructions::{Interrupt, InterruptResult},
    interpreter::{Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError},
};
use std::env;
use std::fs;
use std::process::ExitCode;
use std::time::Instant;

/// The Entry file used when the command line names none.
const DEFAULT_ENTRY: &str = "code-to-run.asm";

/// What the user asked the interpreter to do.
enum Mode {
    /// Run to the end, keeping a history.
    Run,
    /// One instruction at a time, keeping a history.
    Step,
    /// Run to the end with no history, and time it.
    Benchmark,
}

/// What a step of `--step` mode does next.
enum StepKind {
    Step,
    Undo,
    Print,
    Stop,
}

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let flags: Vec<&str> = arguments
        .iter()
        .map(String::as_str)
        .filter(|argument| argument.starts_with("--"))
        .collect();
    let path = arguments
        .iter()
        .find(|argument| !argument.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| DEFAULT_ENTRY.to_string());

    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("{}: {}", path, e);
            return ExitCode::FAILURE;
        }
    };

    // A Project path is root relative with no leading `/` (CONTEXT.md, "File"),
    // so an absolute command-line path is not one and `Files` would normalise
    // it into a path that names no file on disk. The source is filed under the
    // normalised path the Assembler needs, and every Diagnostic is printed
    // against the path the user typed, so that both halves of the output name
    // the same file and an editor can follow the line.
    let entry = normalise_path(&path);
    let mut files = Files::new();
    files.insert_text(&entry, source);
    let assembly = assemble(&files, &entry);
    for diagnostic in &assembly.diagnostics {
        print_diagnostic(diagnostic, &entry, &path);
    }
    let program = match assembly.program {
        Some(program) => program,
        None => {
            eprintln!(
                "\n{} did not assemble: {} error(s).",
                path,
                assembly
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.is_error())
                    .count()
            );
            return ExitCode::FAILURE;
        }
    };

    if flags.contains(&"--show-program") {
        println!("\n----ASSEMBLED-PROGRAM----\n");
        print_program(&program);
    }

    let mode = if flags.contains(&"--step") {
        Mode::Step
    } else if flags.contains(&"--benchmark") {
        Mode::Benchmark
    } else {
        Mode::Run
    };
    let options = match mode {
        Mode::Benchmark => InterpreterOptions {
            keep_history: false,
            history_size: 0,
        },
        _ => InterpreterOptions {
            keep_history: true,
            ..Default::default()
        },
    };
    let start = Instant::now();
    let mut interpreter = Interpreter::new(program, Some(options));
    match mode {
        Mode::Run | Mode::Benchmark => run_to_the_end(&mut interpreter, &entry, &path),
        Mode::Step => step_through(&mut interpreter, &entry, &path),
    }
    if !flags.contains(&"--no-debug") {
        interpreter.debug_status();
        println!("\nExecution took: {:?}", start.elapsed());
    }
    ExitCode::SUCCESS
}

/// One Diagnostic, as `file:line:column: severity: message`, with its hint and
/// its related locations under it.
///
/// The line and the column are 1-based here and 0-based in the
/// [`Location`](s68k::assembler::source::Location) itself, because that is what
/// an editor and every other command line tool count from.
fn print_diagnostic(diagnostic: &Diagnostic, entry: &str, on_disk: &str) {
    let name = |file: &'_ str| -> String {
        match file == entry {
            true => on_disk.to_string(),
            false => file.to_string(),
        }
    };
    let location = &diagnostic.location;
    println!(
        "{}:{}:{}: {}: {}",
        name(&location.file),
        location.line + 1,
        location.column + 1,
        diagnostic.severity.as_str(),
        diagnostic.message()
    );
    if let Some(hint) = diagnostic.hint() {
        println!("    hint: {}", hint);
    }
    for (related, message) in &diagnostic.related {
        println!(
            "    {}:{}:{}: {}",
            name(&related.file),
            related.line + 1,
            related.column + 1,
            message
        );
    }
}

/// Every assembled instruction, its address and the line it came from.
fn print_program(program: &Program) {
    println!("entry point: ${:x}", program.entry());
    for instruction in program.instructions() {
        println!(
            "${:08x}  {}:{}  {}",
            instruction.address,
            instruction.location.file,
            instruction.location.line + 1,
            instruction.source.trim()
        );
    }
}

/// A runtime error, with the line the instruction that raised it was written
/// on.
///
/// A runtime error is not a Diagnostic (CONTEXT.md, "Runtime error"): it is
/// attributed to the instruction, and the instruction knows where it came from,
/// which is what makes `chk`, `trapv` and `illegal` readable on a command line.
fn print_runtime_error(
    interpreter: &Interpreter,
    error: &RuntimeError,
    entry: &str,
    on_disk: &str,
) {
    println!("Runtime error: {:?}", error);
    let address = interpreter.get_current_instruction_address();
    if let Some(instruction) = interpreter.get_instruction_at(address) {
        let file = match instruction.location.file == entry {
            true => on_disk,
            false => instruction.location.file.as_str(),
        };
        println!(
            "    at {}:{}: {}",
            file,
            instruction.location.line + 1,
            instruction.source.trim()
        );
    }
}

/// Runs until the program terminates, answering every interrupt on the way.
fn run_to_the_end(interpreter: &mut Interpreter, entry: &str, on_disk: &str) {
    while !interpreter.has_terminated() {
        let status = match interpreter.run() {
            Ok(status) => status,
            Err(e) => {
                print_runtime_error(interpreter, &e, entry, on_disk);
                return;
            }
        };
        match status {
            InterpreterStatus::Interrupt => {
                let interrupt = interpreter
                    .get_current_interrupt()
                    .expect("an interrupt to answer");
                handle_interrupt(interpreter, &interrupt);
            }
            InterpreterStatus::TerminatedWithException => {
                println!("Program Terminated with exception");
            }
            _ => {}
        }
    }
}

/// One instruction at a time, until the program terminates or the input ends.
fn step_through(interpreter: &mut Interpreter, entry: &str, on_disk: &str) {
    println!("D for step, A for undo, S for print, Q for quit");
    while !interpreter.has_terminated() {
        match ask_step_kind() {
            StepKind::Step => {
                if let Err(e) = interpreter.step() {
                    print_runtime_error(interpreter, &e, entry, on_disk);
                    return;
                }
                match interpreter.get_next_instruction() {
                    Some(instruction) => println!("{}", instruction.source.trim()),
                    None => println!("(no instruction at ${:x})", interpreter.get_pc()),
                }
            }
            StepKind::Undo => {
                if let Err(e) = interpreter.undo() {
                    println!("{:?}", e);
                }
            }
            StepKind::Print => interpreter.debug_status(),
            StepKind::Stop => break,
        }
        if *interpreter.get_status() == InterpreterStatus::Interrupt {
            let interrupt = interpreter
                .get_current_interrupt()
                .expect("an interrupt to answer");
            handle_interrupt(interpreter, &interrupt);
        }
        if *interpreter.get_status() == InterpreterStatus::TerminatedWithException {
            println!("Program Terminated with exception");
        }
    }
}

/// Reads one key of `--step` mode.
///
/// Anything but the four keys stops the run, the end of the input included: a
/// terminal that has nothing left to give answers the empty line for ever, and
/// a question asked for ever is the loop this replaced.
fn ask_step_kind() -> StepKind {
    match Term::stdout().read_line() {
        Ok(line) => match line.trim().to_uppercase().as_str() {
            "D" => StepKind::Step,
            "A" => StepKind::Undo,
            "S" => StepKind::Print,
            _ => StepKind::Stop,
        },
        Err(_) => StepKind::Stop,
    }
}

/// An unsigned number in a base of 2 to 36, digits `0` to `9` then `a` to `z`,
/// which is what task 15 displays.
fn in_base(value: u32, base: u8) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let base = base as u32;
    if value == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    let mut left = value;
    while left > 0 {
        digits.push(DIGITS[(left % base) as usize]);
        left /= base;
    }
    digits.reverse();
    String::from_utf8(digits).expect("the digits are ASCII")
}

/// Answers one interrupt from the terminal: the display tasks print, the input
/// tasks read a line, and everything a terminal cannot do — the graphics tasks
/// — is reported and ends the program.
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
            let char = Term::stdout().read_char().unwrap_or('\0');
            interpreter
                .answer_interrupt(InterruptResult::ReadChar(char))
                .unwrap();
        }
        Interrupt::ReadNumber => {
            let line = Term::stdout().read_line().unwrap_or_default();
            let number = line.trim().parse::<i32>().unwrap_or(0);
            interpreter
                .answer_interrupt(InterruptResult::ReadNumber(number))
                .unwrap();
        }
        Interrupt::ReadKeyboardString => {
            let string = Term::stdout().read_line().unwrap_or_default();
            interpreter
                .answer_interrupt(InterruptResult::ReadKeyboardString(string))
                .unwrap();
        }
        Interrupt::DisplayNumberInBase { value, base } => {
            print!("{}", in_base(*value, *base));
            interpreter
                .answer_interrupt(InterruptResult::DisplayNumberInBase)
                .unwrap();
        }
        Interrupt::DisplaySignedNumberInField { value, width } => {
            print!("{:>width$}", value, width = *width as usize);
            interpreter
                .answer_interrupt(InterruptResult::DisplaySignedNumberInField)
                .unwrap();
        }
        Interrupt::DisplayStringAndNumber { string, number } => {
            print!("{}{}", string, number);
            interpreter
                .answer_interrupt(InterruptResult::DisplayStringAndNumber)
                .unwrap();
        }
        Interrupt::DisplayStringAndReadNumber(string) => {
            print!("{}", string);
            let line = Term::stdout().read_line().unwrap_or_default();
            let number = line.trim().parse::<i32>().unwrap_or(0);
            interpreter
                .answer_interrupt(InterruptResult::DisplayStringAndReadNumber(number))
                .unwrap();
        }
        Interrupt::Terminate => {
            interpreter
                .answer_interrupt(InterruptResult::Terminate)
                .unwrap();
        }
        Interrupt::Delay(_) => {
            interpreter
                .answer_interrupt(InterruptResult::Delay)
                .unwrap();
        }
        _ => {
            println!("Unhandled interrupt: {:?}", interrupt);
            interpreter
                .answer_interrupt(InterruptResult::Terminate)
                .unwrap();
        }
    }
}
