//! The command line front end: assemble a Project and run it.
//!
//! It is the quickest way to see what the Assembler and the Interpreter make
//! of a program without the editor: `cargo run -- program.asm` assembles it,
//! prints every Diagnostic as `file:line:column: severity: message` — with its
//! hint and its related locations under it, the `include` lines among them —
//! and, when none of them is an error, runs it.
//!
//! The Project is the **directory the Entry file is in**, read as far down as
//! it goes ([`read_project`]), so `cargo run -- dir/main.asm` assembles the
//! `include` and `incbin` lines of `dir/main.asm` against the Files beside it.
//!
//! ```text
//! Usage: s68k [FILE] [OPTIONS]
//!
//!   FILE              the Entry file to assemble (default code-to-run.asm);
//!                     its directory is the project
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
use std::collections::BTreeMap;
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

    let (files, entry, on_disk) = match read_project(&path) {
        Ok(project) => project,
        Err((path, e)) => {
            eprintln!("{}: {}", path, e);
            return ExitCode::FAILURE;
        }
    };
    let assembly = assemble(&files, &entry);
    for diagnostic in &assembly.diagnostics {
        print_diagnostic(diagnostic, &on_disk);
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
        print_program(&program, &on_disk);
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
        Mode::Run | Mode::Benchmark => run_to_the_end(&mut interpreter, &on_disk),
        Mode::Step => step_through(&mut interpreter, &on_disk),
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
fn print_diagnostic(diagnostic: &Diagnostic, on_disk: &BTreeMap<String, String>) {
    let name = |file: &'_ str| -> String {
        on_disk
            .get(file)
            .cloned()
            .unwrap_or_else(|| file.to_string())
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
fn print_program(program: &Program, on_disk: &BTreeMap<String, String>) {
    println!("entry point: ${:x}", program.entry());
    for instruction in program.instructions() {
        println!(
            "${:08x}  {}:{}  {}",
            instruction.address,
            on_disk
                .get(&instruction.location.file)
                .unwrap_or(&instruction.location.file),
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
    on_disk: &BTreeMap<String, String>,
) {
    println!("Runtime error: {:?}", error);
    let address = interpreter.get_current_instruction_address();
    if let Some(instruction) = interpreter.get_instruction_at(address) {
        let file = on_disk
            .get(&instruction.location.file)
            .map(String::as_str)
            .unwrap_or(instruction.location.file.as_str());
        println!(
            "    at {}:{}: {}",
            file,
            instruction.location.line + 1,
            instruction.source.trim()
        );
        for site in &instruction.include_chain {
            let file = on_disk
                .get(&site.file)
                .map(String::as_str)
                .unwrap_or(site.file.as_str());
            println!("    included from {}:{}", file, site.line + 1);
        }
    }
}

/// The extensions the command line reads as source; every other File is bytes.
///
/// A Project holds text Files and binary Files (CONTEXT.md, "File"), and a
/// directory on disk says which is which only by the way its Files are named.
/// These five are what the asm-editor and EASy68K call M68K source; anything
/// else is what `incbin` is for.
const SOURCE_EXTENSIONS: [&str; 5] = ["asm", "x68", "m68k", "s", "inc"];

/// Directories the walk does not enter, on top of every name starting with `.`.
///
/// They are build output, never part of a program, and the first of them is
/// 3.6 GB in this repository — which is what `cargo run` with no argument would
/// otherwise read, the default Entry file sitting beside it.
const SKIPPED_DIRECTORIES: [&str; 2] = ["target", "node_modules"];

/// How many Files the command line reads into one Project.
const MAX_PROJECT_FILES: usize = 1000;

/// How many bytes the command line reads into one Project.
const MAX_PROJECT_BYTES: u64 = 32 * 1024 * 1024;

/// Read the Entry file's directory as the Project.
///
/// The Assembler assembles a **Project**, a map of root-relative paths
/// (CONTEXT.md, "File"), and the command line has a directory instead. The
/// project root is the directory the Entry file is in, and every File under it
/// is one File of the Project, named by its path inside it: `cargo run --
/// dir/main.asm` assembles `dir/main.asm` as `main.asm`, and its
/// `include 'lib/io.x68'` finds `dir/lib/io.x68`.
///
/// The whole directory goes in, and not only the Files the `include` lines
/// name, because the Assembler is the one that resolves a name — beside the
/// including File first, at the project root second — and because its
/// `unreadable_file` offers the closest paths **of the Project**: a Project
/// scanned from the `include` lines could neither follow the second of those
/// two rules nor ever suggest the File that was meant. What it costs is a walk
/// with two rules of its own: a directory whose name starts with `.` or that is
/// [build output](SKIPPED_DIRECTORIES) is not entered, and it reads no more
/// than [`MAX_PROJECT_FILES`] Files and [`MAX_PROJECT_BYTES`] bytes, saying so
/// once when it leaves something out, because a directory on disk is not a
/// Project and nothing promises that it is small. Source Files are read first
/// and everything else with what is left, so that a directory of assets beside
/// the program cannot cost it its library.
///
/// It answers the Files, the path of the Entry file inside them, and the map
/// back to the paths on disk, which is what every message prints: a Project
/// path has no leading `/` and is relative to the root, so an absolute
/// command-line path is not one, and both halves of the output have to name the
/// same file.
#[allow(clippy::type_complexity)]
fn read_project(
    entry: &str,
) -> Result<(Files, String, BTreeMap<String, String>), (String, std::io::Error)> {
    let (root, name) = match entry.rfind(['/', '\\']) {
        Some(at) => (&entry[..=at], &entry[at + 1..]),
        None => ("", entry),
    };
    let entry_path = normalise_path(name);
    let mut files = Files::new();
    let mut on_disk = BTreeMap::new();
    // The Entry file is read first, and as source whatever it is called: the
    // command line names it, so it is the program. Everything else is decided
    // by its extension.
    let bytes = fs::read(entry).map_err(|e| (entry.to_string(), e))?;
    files.insert_text(&entry_path, as_text(bytes));
    on_disk.insert(entry_path.clone(), entry.to_string());
    let mut walk = Walk {
        root,
        files: &mut files,
        on_disk: &mut on_disk,
        files_read: 1,
        bytes_read: 0,
        deferred: Vec::new(),
        left_out: false,
    };
    walk.enter("");
    walk.read_the_rest();
    if walk.left_out {
        eprintln!(
            "s68k: this directory holds more than {} files or {} MiB, \
             so some of them are not part of the project.",
            MAX_PROJECT_FILES,
            MAX_PROJECT_BYTES / (1024 * 1024)
        );
    }
    Ok((files, entry_path, on_disk))
}

/// The walk that reads a directory into a Project.
struct Walk<'a> {
    /// What to write before a project path to reach it on disk: `""` or
    /// `dir/`.
    root: &'a str,
    files: &'a mut Files,
    on_disk: &'a mut BTreeMap<String, String>,
    files_read: usize,
    bytes_read: u64,
    /// The Files that are not source, kept back until every source File has had
    /// its share of the budget.
    deferred: Vec<String>,
    /// Whether the budget left a File out, which is one message however many.
    left_out: bool,
}

impl Walk<'_> {
    /// Read one directory of the Project, `relative` to its root, and the
    /// directories under it.
    fn enter(&mut self, relative: &str) {
        let directory = format!("{}{}", self.root, relative);
        let read = fs::read_dir(match directory.is_empty() {
            true => ".",
            false => directory.trim_end_matches('/'),
        });
        let Ok(read) = read else { return };
        let mut entries: Vec<fs::DirEntry> = read.flatten().collect();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = match relative.is_empty() {
                true => name.clone(),
                false => format!("{relative}/{name}"),
            };
            // A symbolic link is neither, which is what keeps a link back up
            // the tree from being a loop.
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => {
                    if !SKIPPED_DIRECTORIES.contains(&name.as_str()) {
                        self.enter(&path);
                    }
                }
                Ok(kind) if kind.is_file() => match is_source(&path) {
                    true => self.read(&path),
                    false => self.defer(path),
                },
                _ => {}
            }
        }
    }

    /// Keep one File that is not source back, to be read with what is left of
    /// the budget once every source File has been read.
    fn defer(&mut self, path: String) {
        match self.deferred.len() < MAX_PROJECT_FILES {
            true => self.deferred.push(path),
            false => self.left_out = true,
        }
    }

    /// Read the Files that were kept back, in the order they were found.
    fn read_the_rest(&mut self) {
        for path in std::mem::take(&mut self.deferred) {
            self.read(&path);
        }
    }

    /// Read one File into the Project, unless the budget has no room for it.
    ///
    /// A File that does not fit is left out and the walk goes on, so that one
    /// large file beside the program does not cost the program its library.
    fn read(&mut self, path: &str) {
        let path = normalise_path(path);
        if self.on_disk.contains_key(&path) {
            return;
        }
        let source_path = format!("{}{}", self.root, path);
        let length = fs::metadata(&source_path)
            .map(|file| file.len())
            .unwrap_or(0);
        if self.files_read >= MAX_PROJECT_FILES || self.bytes_read + length > MAX_PROJECT_BYTES {
            self.left_out = true;
            return;
        }
        let Ok(bytes) = fs::read(&source_path) else {
            return;
        };
        self.files_read += 1;
        self.bytes_read += bytes.len() as u64;
        match is_source(&path) {
            true => self.files.insert_text(&path, as_text(bytes)),
            false => self.files.insert_bytes(&path, bytes),
        };
        self.on_disk.insert(path, source_path);
    }
}

/// Whether a path names a source File, by its extension and nothing else.
fn is_source(path: &str) -> bool {
    let name = match path.rfind('/') {
        Some(at) => &path[at + 1..],
        None => path,
    };
    match name.rfind('.') {
        Some(at) => {
            let extension = name[at + 1..].to_ascii_lowercase();
            SOURCE_EXTENSIONS.contains(&extension.as_str())
        }
        None => false,
    }
}

/// The text of a source File read from disk.
///
/// UTF-8 when the File is UTF-8, and Latin-1 when it is not, because a
/// character is one Latin-1 byte here ([ADR
/// 0004](../docs/adr/0004-characters-are-latin-1-bytes.md)) and a File written
/// by EASy68K holds bytes and not code points: either way `dc.b 'é'` assembles
/// to the one byte $E9 it means. The conversion is total, so no File on disk
/// can stop the command line from assembling.
fn as_text(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => e.into_bytes().iter().map(|&byte| byte as char).collect(),
    }
}

/// Runs until the program pauses or terminates, answering every interrupt on
/// the way.
fn run_to_the_end(interpreter: &mut Interpreter, on_disk: &BTreeMap<String, String>) {
    while !interpreter.has_terminated() {
        let status = match interpreter.run() {
            Ok(status) => status,
            Err(e) => {
                print_runtime_error(interpreter, &e, on_disk);
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
            InterpreterStatus::Paused => {
                println!("Program paused at ${:x}", interpreter.get_pc());
                return;
            }
            _ => {}
        }
    }
}

/// One instruction at a time, until the program terminates or the input ends.
fn step_through(interpreter: &mut Interpreter, on_disk: &BTreeMap<String, String>) {
    println!("D for step, A for undo, S for print, Q for quit");
    while !interpreter.has_terminated() {
        match ask_step_kind() {
            StepKind::Step => {
                if let Err(e) = interpreter.step() {
                    print_runtime_error(interpreter, &e, on_disk);
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

#[cfg(test)]
mod tests {
    use super::*;
    use s68k::assembler::source::FileContent;

    /// Write a directory of Files under the system's temporary directory and
    /// answer its path, with a `/` at the end.
    ///
    /// The name is the test's own, so two tests never share one, and the
    /// directory is removed first in case a failing run left it behind.
    fn a_directory(name: &str, files: &[(&str, &[u8])]) -> String {
        let root = std::env::temp_dir().join(format!("s68k-{name}"));
        let _ = fs::remove_dir_all(&root);
        for (path, content) in files {
            let file = root.join(path);
            fs::create_dir_all(file.parent().expect("a file has a directory"))
                .expect("a writable temporary directory");
            fs::write(&file, content).expect("a writable temporary file");
        }
        format!("{}/", root.display())
    }

    #[test]
    fn the_project_is_the_directory_the_entry_file_is_in() {
        let root = a_directory(
            "project",
            &[
                ("main.asm", b"    include 'lib/io.x68'\n"),
                ("lib/io.x68", b"    nop\n"),
                ("data/sprite.bin", &[1, 2, 3, 4]),
                ("notes.md", b"not source, and read as bytes\n"),
                ("target/build.asm", b"    nop\n"),
                (".hidden/secret.asm", b"    nop\n"),
            ],
        );
        let (files, entry, on_disk) =
            read_project(&format!("{root}main.asm")).expect("the entry file is readable");
        assert_eq!(
            entry, "main.asm",
            "the entry's path is relative to the root"
        );
        assert_eq!(
            files.paths().collect::<Vec<_>>(),
            vec!["data/sprite.bin", "lib/io.x68", "main.asm", "notes.md"],
            "every file under the root but the build output and the dot directory"
        );
        assert!(files.get("lib/io.x68").expect("the library").is_text());
        assert_eq!(
            files.get("data/sprite.bin").and_then(FileContent::as_bytes),
            Some(&[1u8, 2, 3, 4][..]),
            "a file that is not source goes in as the bytes `incbin` wants"
        );
        assert!(
            files
                .get("notes.md")
                .and_then(FileContent::as_bytes)
                .is_some(),
            "the extension decides it, and nothing else"
        );
        assert_eq!(
            on_disk.get("lib/io.x68"),
            Some(&format!("{root}lib/io.x68")),
            "and every project path maps back to the path a message prints"
        );
        let _ = fs::remove_dir_all(root.trim_end_matches('/'));
    }

    #[test]
    fn the_entry_file_is_source_whatever_it_is_called() {
        let root = a_directory("entry", &[("program.txt", b"    nop\n")]);
        let (files, entry, _) =
            read_project(&format!("{root}program.txt")).expect("the entry file is readable");
        assert_eq!(entry, "program.txt");
        assert!(
            files.get("program.txt").expect("the entry file").is_text(),
            "the command line named it, so it is the program"
        );
        let _ = fs::remove_dir_all(root.trim_end_matches('/'));
    }

    #[test]
    fn a_source_file_is_named_by_its_extension() {
        for path in ["main.asm", "lib/io.X68", "a.m68k", "boot.s", "macros.inc"] {
            assert!(is_source(path), "{path} is source");
        }
        for path in ["sprite.bin", "notes.md", "lib/README", "a.asm.bak"] {
            assert!(!is_source(path), "{path} is not");
        }
    }

    #[test]
    fn a_source_file_that_is_not_utf_8_is_read_as_latin_1() {
        // `dc.b 'é'` written by EASy68K: one byte, $E9, and not a UTF-8
        // sequence. It has to reach the Assembler as the one character it is.
        assert_eq!(as_text(vec![b'\'', 0xE9, b'\'']), "'é'");
        assert_eq!(as_text("'é'".as_bytes().to_vec()), "'é'");
    }
}
