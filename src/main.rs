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
//!
//! It is a host of the `trap #15` tasks a terminal can do: the text tasks, with
//! the echo of task 12, and **the file tasks, 50 to 59, on the disk**
//! ([`FileHost`]), a path being relative to the project's directory as the
//! editor's are to its project. The file dialog of task 58 asks for a path on
//! the terminal, an empty line cancelling it. Everything else — the graphics,
//! the mouse, the sound — is reported and ends the run.

use console::Term;
use s68k::assembler::assemble;
use s68k::assembler::diagnostics::Diagnostic;
use s68k::assembler::program::Program;
use s68k::assembler::source::{normalise_path, Files};
use s68k::{
    charset,
    instructions::{FileExistence, InputSettings, Interrupt, InterruptResult, OpenedFile},
    interpreter::{Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError, Termination},
};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
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
    //the project's directory, which is where a program's relative paths start
    let root = match Path::new(&path).parent() {
        Some(directory) if !directory.as_os_str().is_empty() => directory.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut host = FileHost::new(root);
    match mode {
        Mode::Run | Mode::Benchmark => run_to_the_end(&mut interpreter, &mut host, &on_disk),
        Mode::Step => step_through(&mut interpreter, &mut host, &on_disk),
    }
    if let Some(termination) = interpreter.get_termination() {
        println!("\n{}", describe_termination(termination));
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
/// UTF-8 when the File is UTF-8, and Windows-1252 when it is not, because a
/// character is one Windows-1252 byte here ([ADR
/// 0004](../docs/adr/0004-characters-are-latin-1-bytes.md)) and a File written
/// by EASy68K holds bytes in that code page and not code points: either way
/// `dc.b 'é'` assembles to the one byte $E9 it means, and `dc.b '€'` to $80.
/// The conversion is total, so no File on disk can stop the command line from
/// assembling.
fn as_text(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => charset::decode(e.as_bytes()),
    }
}

/// Runs until the program pauses or terminates, answering every interrupt on
/// the way.
fn run_to_the_end(
    interpreter: &mut Interpreter,
    host: &mut FileHost,
    on_disk: &BTreeMap<String, String>,
) {
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
                if let Err(e) = handle_interrupt(interpreter, host, &interrupt) {
                    print_runtime_error(interpreter, &e, on_disk);
                    return;
                }
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
fn step_through(
    interpreter: &mut Interpreter,
    host: &mut FileHost,
    on_disk: &BTreeMap<String, String>,
) {
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
            if let Err(e) = handle_interrupt(interpreter, host, &interrupt) {
                print_runtime_error(interpreter, &e, on_disk);
                return;
            }
        }
    }
}

/// Why the program ended, in a line: what the command line prints after a run.
fn describe_termination(termination: &Termination) -> String {
    match termination {
        Termination::TerminateTask => "The program ended with task 9.".to_string(),
        Termination::EndOfProgram => "The program ran past its last instruction.".to_string(),
        Termination::TerminatedByHost => {
            "The program was ended: the command line cannot do a task it asked for.".to_string()
        }
        Termination::Exception(error) => {
            format!("The program ended with an exception: {:?}", error)
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

/// One line typed at the terminal, as the read tasks are answered: the line
/// without its Enter, which the Interpreter reads with EASy68K's rules. The end
/// of the input is the empty line.
///
/// With the echo of task 12 off, what is typed is not shown, and the terminal
/// still moves to the next line at Enter, as EASy68K does.
fn read_line(settings: InputSettings) -> String {
    // the prompt a display task printed without a new line has to be seen first
    let _ = std::io::stdout().flush();
    let terminal = Term::stdout();
    match settings.echo {
        true => terminal.read_line(),
        false => terminal.read_secure_line(),
    }
    .unwrap_or_default()
}

/// One key typed at the terminal, for task 5, echoed as EASy68K echoes it: the
/// key, and for Enter a carriage return with a line feed when task 16 left it
/// on. Enter reads as `'\n'`, which the Interpreter stores as EASy68K's $0D.
fn read_key(settings: InputSettings) -> char {
    let _ = std::io::stdout().flush();
    let key = Term::stdout().read_char().unwrap_or('\0');
    if settings.echo {
        match key {
            '\n' | '\r' if settings.line_feed => print!("\r\n"),
            '\n' | '\r' => print!("\r"),
            key => print!("{}", key),
        }
        let _ = std::io::stdout().flush();
    }
    key
}

/// The command line's side of the file tasks: EASy68K's eight files, on the
/// disk.
///
/// A path is taken relative to the project's directory, so `cargo run --
/// dir/main.asm` that opens `scores.txt` opens `dir/scores.txt`, which is what
/// the editor does with a path relative to its project; an absolute path is
/// taken as it is. Each task does what EASy68K's `SIMOPS2.CPP` does with the C
/// library, and the Interpreter turns the outcome into the result codes:
///
/// * task 51 opens for reading and writing, and for reading only when the
///   file cannot be written;
/// * task 52 creates the file, or empties it;
/// * a file number goes to the lowest of the eight that is free;
/// * a directory is not a file, which Windows' `fopen` agrees with.
struct FileHost {
    root: PathBuf,
    files: Vec<Option<fs::File>>,
}

impl FileHost {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            files: (0..8).map(|_| None).collect(),
        }
    }

    fn path(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    /// The lowest file number with no file open on it.
    fn free(&self) -> Option<usize> {
        self.files.iter().position(Option::is_none)
    }

    /// A file that is a file and not a directory, which `open` reaches too.
    fn a_file(file: fs::File) -> Option<fs::File> {
        match file.metadata() {
            Ok(metadata) if metadata.is_file() => Some(file),
            _ => None,
        }
    }

    fn open(&mut self, path: &str) -> Option<OpenedFile> {
        let slot = self.free()?;
        let path = self.path(path);
        let (file, read_only) = match fs::OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => (file, false),
            Err(_) => (fs::File::open(&path).ok()?, true),
        };
        self.files[slot] = Some(Self::a_file(file)?);
        Some(OpenedFile {
            handle: slot as u8,
            read_only,
        })
    }

    fn create(&mut self, path: &str) -> Option<u8> {
        let slot = self.free()?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(self.path(path))
            .ok()?;
        self.files[slot] = Some(Self::a_file(file)?);
        Some(slot as u8)
    }

    /// At most `count` bytes from the file's position: fewer at the end of the
    /// file, and none past it.
    fn read(&mut self, handle: u8, count: u32) -> Option<Vec<u8>> {
        let file = self.files.get_mut(handle as usize)?.as_mut()?;
        let mut bytes = Vec::new();
        Read::by_ref(file)
            .take(count as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        Some(bytes)
    }

    fn write(&mut self, handle: u8, bytes: &[u8]) -> bool {
        match self.files.get_mut(handle as usize) {
            Some(Some(file)) => file.write_all(bytes).is_ok(),
            _ => false,
        }
    }

    fn seek(&mut self, handle: u8, offset: u32) -> bool {
        match self.files.get_mut(handle as usize) {
            Some(Some(file)) => file.seek(SeekFrom::Start(offset as u64)).is_ok(),
            _ => false,
        }
    }

    fn close(&mut self, handle: u8) -> bool {
        match self.files.get_mut(handle as usize) {
            Some(slot) => slot.take().is_some(),
            None => false,
        }
    }

    fn close_all(&mut self) -> bool {
        self.files.iter_mut().for_each(|slot| *slot = None);
        true
    }

    fn delete(&mut self, path: &str) -> bool {
        fs::remove_file(self.path(path)).is_ok()
    }

    /// What task 59 finds: a file that opens for writing, one that opens only
    /// for reading, or nothing that opens as a file.
    fn exists(&self, path: &str) -> FileExistence {
        let path = self.path(path);
        if path.is_dir() {
            return FileExistence::Missing;
        }
        if fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .is_ok()
        {
            FileExistence::Writable
        } else if fs::File::open(&path).is_ok() {
            FileExistence::ReadOnly
        } else {
            FileExistence::Missing
        }
    }
}

/// Answers one interrupt from the terminal and the disk: the display tasks
/// print the text the Interpreter formatted, the read tasks hand it the line or
/// the key that was typed, the file tasks are done by the [`FileHost`], and
/// everything a terminal cannot do — the graphics, the mouse and the sound — is
/// reported and ends the program.
///
/// It answers the error of an answer the Interpreter refused, which leaves the
/// interrupt pending, so the caller stops there.
fn handle_interrupt(
    interpreter: &mut Interpreter,
    host: &mut FileHost,
    interrupt: &Interrupt,
) -> Result<(), RuntimeError> {
    let settings = interpreter.get_input_settings();
    let answer = match interrupt {
        Interrupt::DisplayStringWithCRLF(text) => {
            println!("{}", text);
            InterruptResult::DisplayStringWithCRLF
        }
        Interrupt::DisplayStringWithoutCRLF(text) => {
            print!("{}", text);
            InterruptResult::DisplayStringWithoutCRLF
        }
        Interrupt::DisplayNumber(text) => {
            print!("{}", text);
            InterruptResult::DisplayNumber
        }
        Interrupt::DisplayNumberInBase(text) => {
            print!("{}", text);
            InterruptResult::DisplayNumberInBase
        }
        Interrupt::DisplaySignedNumberInField(text) => {
            print!("{}", text);
            InterruptResult::DisplaySignedNumberInField
        }
        Interrupt::DisplayStringAndNumber(text) => {
            print!("{}", text);
            InterruptResult::DisplayStringAndNumber
        }
        Interrupt::DisplayChar(character) => {
            print!("{}", character);
            InterruptResult::DisplayChar
        }
        Interrupt::DisplayStringAndReadNumber(prompt) => {
            print!("{}", prompt);
            InterruptResult::DisplayStringAndReadNumber(read_line(settings))
        }
        Interrupt::ReadNumber => InterruptResult::ReadNumber(read_line(settings)),
        Interrupt::ReadKeyboardString => InterruptResult::ReadKeyboardString(read_line(settings)),
        Interrupt::ReadChar => InterruptResult::ReadChar(read_key(settings)),
        Interrupt::GetTime => InterruptResult::GetTime(0),
        Interrupt::Terminate => InterruptResult::Terminate,
        Interrupt::Delay(_) => InterruptResult::Delay,
        Interrupt::CloseAllFiles => InterruptResult::CloseAllFiles(host.close_all()),
        Interrupt::OpenFile(path) => InterruptResult::OpenFile(host.open(path)),
        Interrupt::NewFile(path) => InterruptResult::NewFile(host.create(path)),
        Interrupt::ReadFile { handle, count } => {
            InterruptResult::ReadFile(host.read(*handle, *count).map(Into::into))
        }
        Interrupt::WriteFile { handle, bytes } => {
            InterruptResult::WriteFile(host.write(*handle, bytes.as_slice()))
        }
        Interrupt::PositionFile { handle, offset } => {
            InterruptResult::PositionFile(host.seek(*handle, *offset))
        }
        Interrupt::CloseFile(handle) => InterruptResult::CloseFile(host.close(*handle)),
        Interrupt::DeleteFile(path) => InterruptResult::DeleteFile(host.delete(path)),
        Interrupt::FileExists(path) => InterruptResult::FileExists(host.exists(path)),
        Interrupt::FileDialog {
            mode,
            title,
            filter,
            path,
        } => {
            print!("{:?} file", mode);
            for (label, text) in [
                ("", title),
                (" matching ", filter),
                (", starting at ", path),
            ] {
                if !text.is_empty() {
                    print!("{}{}", label, text);
                }
            }
            print!(" (an empty line cancels): ");
            let line = read_line(InputSettings::default());
            InterruptResult::FileDialog(match line.trim() {
                "" => None,
                chosen => Some(chosen.to_string()),
            })
        }
        _ => {
            println!("Unhandled interrupt: {:?}", interrupt);
            InterruptResult::Terminate
        }
    };
    interpreter.answer_interrupt(answer)
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
    fn a_source_file_that_is_not_utf_8_is_read_as_windows_1252() {
        // `dc.b 'é'` written by EASy68K: one byte, $E9, and not a UTF-8
        // sequence. It has to reach the Assembler as the one character it is.
        assert_eq!(as_text(vec![b'\'', 0xE9, b'\'']), "'é'");
        assert_eq!(as_text("'é'".as_bytes().to_vec()), "'é'");
        // `€` is $80 in EASy68K's code page, where Latin-1 has a control code
        assert_eq!(as_text(vec![b'\'', 0x80, b'\'']), "'€'");
    }

    /// A [`FileHost`] over a fresh temporary directory.
    fn a_file_host(name: &str) -> (FileHost, String) {
        let root = a_directory(name, &[("in.txt", b"0123456789")]);
        (FileHost::new(PathBuf::from(&root)), root)
    }

    #[test]
    fn the_file_host_reads_writes_and_seeks_on_the_disk() {
        let (mut host, root) = a_file_host("file-host");
        let handle = host.create("out.txt").expect("a new file");
        assert_eq!(handle, 0, "the lowest free number");
        assert!(host.write(handle, b"hello"));
        assert!(host.seek(handle, 1));
        assert_eq!(
            host.read(handle, 10).as_deref(),
            Some(&b"ello"[..]),
            "a short read"
        );
        assert_eq!(
            host.read(handle, 10).as_deref(),
            Some(&b""[..]),
            "the end of the file"
        );
        assert!(host.close(handle));
        assert!(!host.close(handle), "a file closes once");
        assert_eq!(fs::read(format!("{root}out.txt")).unwrap(), b"hello");

        let opened = host.open("in.txt").expect("an existing file");
        assert!(!opened.read_only);
        assert_eq!(host.read(opened.handle, 4).as_deref(), Some(&b"0123"[..]));
        assert_eq!(host.exists("in.txt"), FileExistence::Writable);
        assert!(host.close_all());
        assert!(host.delete("in.txt"));
        assert_eq!(host.exists("in.txt"), FileExistence::Missing);
        assert!(host.open("in.txt").is_none(), "an open does not create");
        assert!(!host.delete("in.txt"));
        let _ = fs::remove_dir_all(root.trim_end_matches('/'));
    }

    #[test]
    fn the_file_host_has_eight_files() {
        let (mut host, root) = a_file_host("file-host-eight");
        for expected in 0..8u8 {
            assert_eq!(host.open("in.txt").map(|file| file.handle), Some(expected));
        }
        assert!(host.open("in.txt").is_none(), "a ninth file does not open");
        assert!(host.create("new.txt").is_none());
        assert!(host.close(3));
        assert_eq!(
            host.create("new.txt"),
            Some(3),
            "the number closed is free again"
        );
        let _ = fs::remove_dir_all(root.trim_end_matches('/'));
    }

    #[test]
    fn a_directory_is_not_a_file() {
        let (mut host, root) = a_file_host("file-host-directory");
        fs::create_dir_all(format!("{root}data")).unwrap();
        assert_eq!(host.exists("data"), FileExistence::Missing);
        assert!(host.open("data").is_none());
        let _ = fs::remove_dir_all(root.trim_end_matches('/'));
    }

    #[test]
    fn a_file_that_cannot_be_written_opens_for_reading_only() {
        let (mut host, root) = a_file_host("file-host-read-only");
        let path = format!("{root}in.txt");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        // an administrator writes a read-only file anyway, and has nothing to show here
        if fs::OpenOptions::new().write(true).open(&path).is_err() {
            assert_eq!(host.exists("in.txt"), FileExistence::ReadOnly);
            let opened = host.open("in.txt").expect("it opens for reading");
            assert!(opened.read_only);
            assert!(!host.write(opened.handle, b"x"), "and a write fails");
            assert_eq!(host.read(opened.handle, 2).as_deref(), Some(&b"01"[..]));
        }
        let _ = fs::remove_dir_all(root.trim_end_matches('/'));
    }
}
