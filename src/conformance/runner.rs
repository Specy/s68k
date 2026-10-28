//! The runner: one single-step test through an
//! [`Interpreter::from_state`], and one of Musashi's self-checking programs
//! through an [`Interpreter::from_image`].
//!
//! A test sets the state, runs one instruction and compares D0–D7, A0–A6,
//! both stack pointers, SR, the program counter and every RAM word the final
//! state lists. It also checks that the step
//! wrote no byte the final state leaves out. The length in cycles is compared
//! only when [`CHECK_LENGTH`] is on.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

use super::skips::{self, Skip, CHECK_LENGTH};
use super::suite::{self, FormatError, Test};
use crate::assembler::program::Program;
use crate::debugger::MutationOperation;
use crate::interpreter::{
    Cpu, Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError, ADDRESS_MASK,
};

/// What one test came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Skipped(&'static Skip),
    /// Each difference from the final state, in words.
    Failed(Vec<String>),
}

/// What a file of tests came to.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// The file's name without `.json.bin`, `ADD.b`.
    pub file: String,
    /// The tests run: passed and failed, not skipped.
    pub run: usize,
    pub passed: usize,
    /// The tests skipped, by the name of their [`Skip`].
    pub skipped: BTreeMap<&'static str, usize>,
    /// The failed tests, each name with its differences.
    pub failures: Vec<(String, Vec<String>)>,
    /// The failed tests of a file [`KNOWN_FAILURES`] lists, which do not
    /// count as failures.
    ///
    /// [`KNOWN_FAILURES`]: skips::KNOWN_FAILURES
    pub known: Vec<(String, Vec<String>)>,
}

impl Report {
    /// The tests skipped, whatever the reason.
    pub fn skipped_count(&self) -> usize {
        self.skipped.values().sum()
    }
}

/// Runs every test of a file, `file` being its name without `.json.bin`.
pub fn run_file(file: &str, bytes: &[u8]) -> Result<Report, FormatError> {
    let mut report = Report {
        file: file.to_string(),
        ..Report::default()
    };
    for test in suite::read(bytes)? {
        match run(file, &test) {
            Outcome::Passed => {
                report.run += 1;
                report.passed += 1;
            }
            Outcome::Skipped(skip) => *report.skipped.entry(skip.name).or_default() += 1,
            Outcome::Failed(problems) => {
                report.run += 1;
                match skips::KNOWN_FAILURES.iter().any(|(known, _)| *known == file) {
                    true => report.known.push((test.name.clone(), problems)),
                    false => report.failures.push((test.name.clone(), problems)),
                }
            }
        }
    }
    Ok(report)
}

/// Runs one test of `file`, unless the skip list leaves it out.
pub fn run(file: &str, test: &Test) -> Outcome {
    if let Some(skip) = skips::skip(file, test) {
        return Outcome::Skipped(skip);
    }
    let problems = check(file, test);
    match problems.is_empty() {
        true => Outcome::Passed,
        false => Outcome::Failed(problems),
    }
}

/// The interpreter a test's initial state makes, about to run its
/// instruction: both stack pointers, A7 the one of the state's mode, and the
/// program counter four before the state's, which is MAME's next prefetch.
pub fn interpreter_for(test: &Test) -> Interpreter {
    let initial = &test.initial;
    let cpu = Cpu::from_state(initial.d, initial.a, initial.usp, initial.ssp, initial.sr);
    let pc = initial.pc.wrapping_sub(4) as usize & ADDRESS_MASK;
    let ram: Vec<(usize, u16)> = initial
        .ram
        .iter()
        .map(|&(address, word)| (address as usize, word))
        .collect();
    Interpreter::from_state(cpu, pc, &ram, initial.prefetch)
}

/// Every difference between what the instruction left and the final state.
fn check(file: &str, test: &Test) -> Vec<String> {
    let stepped = quietly(|| {
        let mut interpreter = interpreter_for(test);
        let result = interpreter.step();
        (interpreter, result)
    });
    let (interpreter, result) = match stepped {
        Ok(stepped) => stepped,
        Err(message) => return vec![format!("panicked: {message}")],
    };
    let expected = &test.final_state;
    let mut problems = Vec::new();
    match result {
        Err(error) => problems.push(format!("raised {error:?}")),
        Ok(status) if status != InterpreterStatus::Running => {
            problems.push(format!("ended as {status:?}, {:?}", interpreter.get_termination()))
        }
        Ok(_) => {}
    }
    let registers = interpreter.get_cpu().get_register_values();
    let mut compare = |name: String, found: u32, wanted: u32| {
        if found != wanted {
            problems.push(format!("{name} is ${found:08X}, expected ${wanted:08X}"));
        }
    };
    for (index, &wanted) in expected.d.iter().enumerate() {
        compare(format!("D{index}"), registers[index], wanted);
    }
    for (index, &wanted) in expected.a.iter().enumerate() {
        compare(format!("A{index}"), registers[8 + index], wanted);
    }
    let cpu = interpreter.get_cpu();
    compare("USP".to_string(), cpu.get_usp(), expected.usp);
    compare("SSP".to_string(), cpu.get_ssp(), expected.ssp);
    let mask = !skips::relaxed_flags(file);
    let sr = interpreter.get_sr();
    if sr & mask != expected.sr & mask {
        problems.push(format!(
            "SR is ${sr:04X} ({}), expected ${:04X} ({})",
            flags(sr),
            expected.sr,
            flags(expected.sr)
        ));
    }
    let pc = (interpreter.get_pc() & ADDRESS_MASK) as u32;
    let wanted_pc = expected.pc.wrapping_sub(4) & ADDRESS_MASK as u32;
    if pc != wanted_pc {
        problems.push(format!("PC is ${pc:06X}, expected ${wanted_pc:06X}"));
    }
    for &(address, wanted) in &expected.ram {
        let found = word_at(&interpreter, address as usize);
        if found != wanted {
            problems.push(format!("${address:06X} holds ${found:04X}, expected ${wanted:04X}"));
        }
    }
    let listed: BTreeSet<usize> = expected
        .ram
        .iter()
        .map(|&(address, _)| address as usize & ADDRESS_MASK)
        .collect();
    for address in written_bytes(&interpreter) {
        if !listed.contains(&(address & !1)) {
            problems.push(format!("wrote ${address:06X}, which the final state does not list"));
        }
    }
    if CHECK_LENGTH && !skips::relaxed_length(file) {
        problems.push(format!(
            "the length is {} cycles and s68k counts none",
            test.length
        ));
    }
    problems
}

/// The word at `address`, its two bytes each reduced to the 24 bits of the
/// bus.
fn word_at(interpreter: &Interpreter, address: usize) -> u16 {
    let memory = interpreter.get_memory();
    let byte = |address: usize| memory.read_bytes(address & ADDRESS_MASK, 1).unwrap()[0];
    u16::from_be_bytes([byte(address), byte(address + 1)])
}

/// Every byte the last step wrote to memory, from its mutations.
fn written_bytes(interpreter: &Interpreter) -> BTreeSet<usize> {
    let mut written = BTreeSet::new();
    for step in interpreter.get_last_steps(1) {
        for mutation in step.get_mutations() {
            let (address, length) = match mutation {
                MutationOperation::WriteMemory { address, size, .. } => (*address, size.to_bytes()),
                MutationOperation::WriteMemoryBytes { address, new, .. } => (*address, new.len()),
                _ => continue,
            };
            written.extend((0..length).map(|offset| (address + offset) & ADDRESS_MASK));
        }
    }
    written
}

/// The condition codes of `sr`, as `XNZVC` with a dot for each clear one.
fn flags(sr: u16) -> String {
    "XNZVC"
        .chars()
        .enumerate()
        .map(|(index, name)| match sr & (0x10 >> index) != 0 {
            true => name,
            false => '.',
        })
        .collect()
}

thread_local! {
    /// Whether this thread is inside [`quietly`], whose panics are answered
    /// as failures rather than printed.
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f`, answering a panic as its message: a panic is a failure of the
/// test, and the next test runs. The default hook stays in force for every
/// other thread and outside `f`.
fn quietly<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !QUIET.with(Cell::get) {
                previous(info);
            }
        }));
    });
    QUIET.with(|quiet| quiet.set(true));
    let result = panic::catch_unwind(AssertUnwindSafe(f));
    QUIET.with(|quiet| quiet.set(false));
    result.map_err(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
            .unwrap_or_else(|| "a panic with no message".to_string())
    })
}

/// Where Musashi's programs are linked and start.
pub const MUSASHI_LOAD_ADDRESS: usize = 0x10000;
/// A long written here fails the program, `TEST_FAIL_REG`.
pub const MUSASHI_FAIL: usize = 0x100000;
/// A long written here passes it, `TEST_PASS_REG`.
pub const MUSASHI_PASS: usize = 0x100004;
/// The supervisor stack pointer the test driver's reset vector holds.
pub const MUSASHI_STACK: u32 = 0x3F0;
/// The most instructions a program may run: the longest runs about a
/// million.
const MUSASHI_LIMIT: usize = 50_000_000;

/// Runs one of Musashi's self-checking programs, the bytes of its `.bin`,
/// and answers whether it passed: it wrote `TEST_PASS_REG` and never
/// `TEST_FAIL_REG`. The writes are seen through watched memory. Each program
/// ends with `stop #$2700` after its verdict, which s68k ends the program on
/// until C8; that end is expected.
pub fn run_musashi(bytes: &[u8]) -> Result<(), String> {
    let program = Program::from_image(MUSASHI_LOAD_ADDRESS, bytes.to_vec(), MUSASHI_LOAD_ADDRESS);
    let options = InterpreterOptions {
        keep_history: false,
        history_size: 0,
    };
    let run = quietly(|| {
        let mut interpreter = Interpreter::from_image(program, MUSASHI_STACK, Some(options));
        interpreter
            .watch_memory(MUSASHI_FAIL, MUSASHI_PASS + 4)
            .map_err(|error| format!("{error:?}"))?;
        let mut passed = false;
        let mut executed = 0;
        loop {
            let status = match interpreter.run_with_limit(MUSASHI_LIMIT - executed) {
                Ok(status) => status,
                Err(RuntimeError::ExecutionLimit(_)) => {
                    return Err(format!("ran {MUSASHI_LIMIT} instructions without ending"))
                }
                Err(error) if passed && is_the_final_stop(&error) => return Ok(()),
                Err(error) => {
                    return Err(format!(
                        "raised {error:?} at ${:06X}",
                        interpreter.get_current_instruction_address()
                    ))
                }
            };
            executed = interpreter.get_executed_instructions() as usize;
            for write in interpreter.take_watched_writes() {
                if write.address == MUSASHI_FAIL {
                    return Err(format!(
                        "wrote TEST_FAIL_REG at ${:06X}",
                        interpreter.get_current_instruction_address()
                    ));
                }
                passed |= write.address == MUSASHI_PASS;
            }
            if interpreter.has_terminated() {
                return match passed {
                    true => Ok(()),
                    false => Err(format!("ended as {:?} without passing", status)),
                };
            }
        }
    });
    run.unwrap_or_else(|message| Err(format!("panicked: {message}")))
}

/// Whether `error` is the `stop` a program ends with, which s68k does not run
/// yet.
fn is_the_final_stop(error: &RuntimeError) -> bool {
    matches!(error, RuntimeError::Unimplemented(text) if text == "`stop`")
}
