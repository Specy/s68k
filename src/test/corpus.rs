//! Golden fixtures of what s68k makes of the corpus in `tests/corpus`.
//!
//! The design record's "Tests" items 1 and 2
//! (`docs/design/assembler-rewrite.md`): the fixtures were taken from the
//! pipeline of 1.4.2 in phase 0, before the Assembler replaced it, so that the
//! rewrite can be held to them.
//!
//! Three tests, one per half of the corpus and one for what running it does:
//!
//! * [`editor_programs`] assembles every program in `tests/corpus/editor` and
//!   snapshots the Program it produces (Entry point, instructions, initial
//!   memory, Labels) as `<stem>.snap`.
//! * [`easy68k_programs_do_not_assemble`] records that the three EASy68K
//!   originals in `tests/corpus/easy68k` do not assemble, and snapshots the
//!   Diagnostics they raise as `<stem>-errors.snap` — the features s68k has not
//!   implemented, and nothing else.
//! * [`editor_programs_run`] runs every `editor/` program under a deterministic
//!   interrupt policy and snapshots where it gets to, as `<stem>-run.snap`.
//!
//! All three are the Assembler's and the Interpreter's: the old pipeline is
//! gone from the crate and nothing here reaches 1.4.2 any more.
//!
//! The fixture is deliberately assembler-agnostic: nothing in it names a Rust
//! type, a field or an internal convention, so a different implementation of the
//! same language can produce the same bytes. `tests/corpus/README.md`,
//! "Fixture format", is the specification; this file is one implementation of
//! it and the two must be kept in step.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::assembler::diagnostics::Diagnostic;
use crate::assembler::program::{MemoryContent, Program};
use crate::assembler::source::Files;
use crate::assembler::symbols::SymbolKind;
use crate::instructions::{
    Condition, Instruction, Interrupt, InterruptResult, KeyStateRequest, KeyStateResult, Operand,
    RegisterOperand, ShiftDirection, Sign, Size, TargetDirection,
};
use crate::interpreter::{Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError};

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

/// One assembled program, as the fixture format of `tests/corpus/README.md`.
#[derive(Serialize)]
struct Fixture {
    /// Entry point, the address the program starts running at.
    entry: String,
    /// Every assembled instruction, in address order.
    instructions: Vec<FixtureInstruction>,
    /// Every run of bytes a data directive puts in memory, in address order.
    memory: Vec<FixtureMemory>,
    /// Every Label, by name.
    labels: BTreeMap<String, FixtureLabel>,
}

#[derive(Serialize)]
struct FixtureInstruction {
    address: String,
    /// 0-based index of the source line the instruction came from.
    line: usize,
    /// Canonical rendering, printed from the instruction and never from source.
    text: String,
}

/// A run of initial memory: `dc`/`dcb` carry their bytes, `ds` only the number
/// of bytes it reserves and does not write.
#[derive(Serialize)]
#[serde(untagged)]
enum FixtureMemory {
    Bytes { address: String, bytes: String },
    Reserved { address: String, reserved: usize },
}

#[derive(Serialize)]
struct FixtureLabel {
    address: String,
    /// 0-based index of the source line the Label is on.
    line: usize,
}

/// Lowercase hex with a `$` prefix and no padding, the fixture's only number format.
fn hex(value: usize) -> String {
    format!("${:x}", value)
}

// ---------------------------------------------------------------------------
// The dump
// ---------------------------------------------------------------------------

/// Assembles `source` with the Assembler, panicking with `name` and the first
/// Diagnostics in the message if it does not assemble.
fn assemble(name: &str, source: &str) -> Program {
    let assembly = crate::assembler::assemble_source(source);
    match assembly.program {
        Some(program) => program,
        None => {
            let errors: Vec<String> = assembly
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .take(5)
                .map(|diagnostic| {
                    format!(
                        "line {}: {} [{}]",
                        diagnostic.location.line + 1,
                        diagnostic.message(),
                        diagnostic.code()
                    )
                })
                .collect();
            panic!("{} did not assemble:\n{}", name, errors.join("\n"))
        }
    }
}

/// Assembles `source` and builds its fixture.
///
/// `name` only names the program in the panic messages.
fn dump(name: &str, source: &str) -> Fixture {
    let program = assemble(name, source);

    let instructions: Vec<FixtureInstruction> = program
        .instructions()
        .iter()
        .map(|instruction| FixtureInstruction {
            address: hex(instruction.address),
            line: instruction.location.line,
            text: print_instruction(&instruction.instruction),
        })
        .collect();

    let memory: Vec<FixtureMemory> = program
        .memory()
        .iter()
        .map(|run| match &run.content {
            MemoryContent::Bytes { bytes } => FixtureMemory::Bytes {
                address: hex(run.address),
                bytes: print_bytes(bytes),
            },
            MemoryContent::Reserved { length } => FixtureMemory::Reserved {
                address: hex(run.address),
                reserved: *length,
            },
        })
        .collect();

    // `labels` is one entry per Label, as `tests/corpus/README.md` has it. The
    // Constants of `equ` are Symbols of the Program now, where 1.4.2 substituted
    // their text and kept none, and they are deliberately not written here:
    // the field is the Labels, and a Constant's value is visible in every
    // instruction that uses it.
    let labels = program
        .symbols()
        .values()
        .filter(|symbol| symbol.kind == SymbolKind::Label)
        .map(|symbol| {
            (
                symbol.name.clone(),
                FixtureLabel {
                    address: hex(symbol.value.max(0) as usize),
                    line: symbol.location.line,
                },
            )
        })
        .collect();

    Fixture {
        entry: hex(program.entry()),
        instructions,
        memory,
        labels,
    }
}

/// Bytes as one run of lowercase hex, two characters a byte, nothing between them.
fn print_bytes(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for byte in data {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

// ---------------------------------------------------------------------------
// The canonical printer
// ---------------------------------------------------------------------------
//
// The rules are written out in `tests/corpus/README.md` under "Fixture format".
// Both matches below are exhaustive with no wildcard arm on purpose: a new
// instruction or addressing mode must fail to compile here rather than be
// printed wrong.

fn size_suffix(size: Size) -> &'static str {
    match size {
        Size::Byte => "b",
        Size::Word => "w",
        Size::Long => "l",
    }
}

/// `d0`..`d7` and `a0`..`a7`; there is no `sp`, the stack pointer prints as `a7`.
fn register(register: &RegisterOperand) -> String {
    match register {
        RegisterOperand::Data(index) => format!("d{}", index),
        RegisterOperand::Address(index) => format!("a{}", index),
    }
}

/// The canonical mnemonic suffix of a condition; `hs`/`lo`/`ra` have none of
/// their own and come out as `cc`/`cs`/`f`.
fn condition(condition: &Condition) -> &'static str {
    match condition {
        Condition::True => "t",
        Condition::False => "f",
        Condition::High => "hi",
        Condition::LowOrSame => "ls",
        Condition::CarryClear => "cc",
        Condition::CarrySet => "cs",
        Condition::NotEqual => "ne",
        Condition::Equal => "eq",
        Condition::OverflowClear => "vc",
        Condition::OverflowSet => "vs",
        Condition::Plus => "pl",
        Condition::Minus => "mi",
        Condition::GreaterThanOrEqual => "ge",
        Condition::LessThan => "lt",
        Condition::GreaterThan => "gt",
        Condition::LessThanOrEqual => "le",
    }
}

fn shift(direction: &ShiftDirection) -> &'static str {
    match direction {
        ShiftDirection::Left => "l",
        ShiftDirection::Right => "r",
    }
}

fn sign(sign: &Sign) -> &'static str {
    match sign {
        Sign::Signed => "s",
        Sign::Unsigned => "u",
    }
}

fn print_operand(operand: &Operand) -> String {
    match operand {
        Operand::Immediate(value) => format!("#${:x}", value),
        Operand::Register(reg) => register(reg),
        Operand::Indirect(index) => format!("(a{})", index),
        Operand::PostIndirect(index) => format!("(a{})+", index),
        Operand::PreIndirect(index) => format!("-(a{})", index),
        Operand::IndirectDisplacement { offset, base } => {
            format!("{}({})", offset, register(base))
        }
        Operand::IndirectIndex {
            base,
            offset,
            index,
        } => format!(
            "{}({},{}.{})",
            offset,
            register(base),
            register(&index.register),
            size_suffix(index.size)
        ),
        Operand::Absolute(address) => hex(*address),
        // A PC-relative Operand prints the displacement the Assembler worked
        // out and not the address the source wrote: the address is where it
        // came from and the displacement is what the Program holds
        // (`tests/corpus/README.md`, "Operands").
        Operand::PcDisplacement { offset } => format!("{}(pc)", offset),
        Operand::PcIndex { offset, index } => format!(
            "{}(pc,{}.{})",
            offset,
            register(&index.register),
            size_suffix(index.size)
        ),
    }
}

/// A `movem` mask as a register list, `d0-d2/a0/a6`: lowest register first,
/// data registers before address registers, two or more consecutive registers
/// of the same file written as a range.
fn print_register_list(mask: u16) -> String {
    fn name(index: usize) -> String {
        if index < 8 {
            format!("d{}", index)
        } else {
            format!("a{}", index - 8)
        }
    }
    let mut groups: Vec<String> = Vec::new();
    let mut index = 0usize;
    while index < 16 {
        if mask & (1 << index) == 0 {
            index += 1;
            continue;
        }
        // A range never crosses from the data registers into the address ones.
        let file_end = if index < 8 { 8 } else { 16 };
        let start = index;
        let mut end = index;
        while end + 1 < file_end && mask & (1 << (end + 1)) != 0 {
            end += 1;
        }
        if end == start {
            groups.push(name(start));
        } else {
            groups.push(format!("{}-{}", name(start), name(end)));
        }
        index = end + 1;
    }
    groups.join("/")
}

fn print_instruction(instruction: &Instruction) -> String {
    match instruction {
        Instruction::ADDA(source, destination, size) => format!(
            "adda.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            register(destination)
        ),
        Instruction::SUBA(source, destination, size) => format!(
            "suba.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            register(destination)
        ),
        Instruction::CMPA(source, destination, size) => format!(
            "cmpa.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            register(destination)
        ),
        Instruction::MOVEA(source, destination, size) => format!(
            "movea.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            register(destination)
        ),
        Instruction::MOVEM {
            direction,
            size,
            registers_mask,
            target,
        } => {
            // The compiler stores the mask bit-reversed when the target is a
            // predecrement, which is how the interpreter wants to read it back;
            // undo that so that the list reads as it was written in source.
            let mask = if matches!(target, Operand::PreIndirect(_)) {
                registers_mask.reverse_bits()
            } else {
                *registers_mask
            };
            let list = print_register_list(mask);
            let target = print_operand(target);
            match direction {
                TargetDirection::ToMemory => {
                    format!("movem.{} {},{}", size_suffix(*size), list, target)
                }
                TargetDirection::FromMemory => {
                    format!("movem.{} {},{}", size_suffix(*size), target, list)
                }
            }
        }
        Instruction::MOVE(source, destination, size) => format!(
            "move.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            print_operand(destination)
        ),
        Instruction::ADD(source, destination, size) => format!(
            "add.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            print_operand(destination)
        ),
        Instruction::SUB(source, destination, size) => format!(
            "sub.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            print_operand(destination)
        ),
        Instruction::ADDQ(value, destination, size) => format!(
            "addq.{} #${:x},{}",
            size_suffix(*size),
            value,
            print_operand(destination)
        ),
        Instruction::MOVEQ(value, destination) => {
            format!("moveq #${:x},{}", value, register(destination))
        }
        Instruction::SUBQ(value, destination, size) => format!(
            "subq.{} #${:x},{}",
            size_suffix(*size),
            value,
            print_operand(destination)
        ),
        Instruction::ADDI(value, destination, size) => format!(
            "addi.{} #${:x},{}",
            size_suffix(*size),
            value,
            print_operand(destination)
        ),
        Instruction::SUBI(value, destination, size) => format!(
            "subi.{} #${:x},{}",
            size_suffix(*size),
            value,
            print_operand(destination)
        ),
        Instruction::ANDI(value, destination, size) => format!(
            "andi.{} #${:x},{}",
            size_suffix(*size),
            value,
            print_operand(destination)
        ),
        Instruction::ORI(value, destination, size) => format!(
            "ori.{} #${:x},{}",
            size_suffix(*size),
            value,
            print_operand(destination)
        ),
        Instruction::EORI(value, destination, size) => format!(
            "eori.{} #${:x},{}",
            size_suffix(*size),
            value,
            print_operand(destination)
        ),
        Instruction::CMPI(value, destination, size) => format!(
            "cmpi.{} #${:x},{}",
            size_suffix(*size),
            value,
            print_operand(destination)
        ),
        Instruction::CMPM(source, destination, size) => format!(
            "cmpm.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            print_operand(destination)
        ),
        Instruction::DIVx(source, destination, kind) => format!(
            "div{} {},{}",
            sign(kind),
            print_operand(source),
            register(destination)
        ),
        Instruction::MULx(source, destination, kind) => format!(
            "mul{} {},{}",
            sign(kind),
            print_operand(source),
            register(destination)
        ),
        Instruction::SWAP(destination) => format!("swap {}", register(destination)),
        Instruction::CLR(destination, size) => {
            format!("clr.{} {}", size_suffix(*size), print_operand(destination))
        }
        Instruction::EXG(first, second) => {
            format!("exg {},{}", register(first), register(second))
        }
        Instruction::LEA(source, destination) => {
            format!("lea {},{}", print_operand(source), register(destination))
        }
        Instruction::PEA(source) => format!("pea {}", print_operand(source)),
        // The extend-flag pair: two data registers or two predecrements, and
        // the size is written because all three are a choice.
        Instruction::ADDX(source, destination, size) => format!(
            "addx.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            print_operand(destination)
        ),
        Instruction::SUBX(source, destination, size) => format!(
            "subx.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            print_operand(destination)
        ),
        Instruction::NEGX(destination, size) => {
            format!("negx.{} {}", size_suffix(*size), print_operand(destination))
        }
        // The three binary coded decimal instructions carry no size: a byte is
        // the only one they have, the way `tas` has only a byte.
        Instruction::ABCD(source, destination) => format!(
            "abcd {},{}",
            print_operand(source),
            print_operand(destination)
        ),
        Instruction::SBCD(source, destination) => format!(
            "sbcd {},{}",
            print_operand(source),
            print_operand(destination)
        ),
        Instruction::NBCD(destination) => format!("nbcd {}", print_operand(destination)),
        Instruction::NEG(destination, size) => {
            format!("neg.{} {}", size_suffix(*size), print_operand(destination))
        }
        Instruction::EXT(destination, from, to) => {
            // `ext` prints with its destination size; the byte to long form,
            // which only `extb` can write, prints as `extb`.
            let mnemonic = match (from, to) {
                (Size::Byte, Size::Long) => "extb",
                (Size::Byte, Size::Byte) => "ext",
                (Size::Byte, Size::Word) => "ext",
                (Size::Word, Size::Byte) => "ext",
                (Size::Word, Size::Word) => "ext",
                (Size::Word, Size::Long) => "ext",
                (Size::Long, Size::Byte) => "ext",
                (Size::Long, Size::Word) => "ext",
                (Size::Long, Size::Long) => "ext",
            };
            format!(
                "{}.{} {}",
                mnemonic,
                size_suffix(*to),
                register(destination)
            )
        }
        Instruction::TST(destination, size) => {
            format!("tst.{} {}", size_suffix(*size), print_operand(destination))
        }
        Instruction::CMP(source, destination, size) => format!(
            "cmp.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            register(destination)
        ),
        Instruction::Bcc(address, cond) => {
            format!("b{} {}", condition(cond), hex(*address as usize))
        }
        Instruction::Scc(destination, cond) => {
            format!("s{} {}", condition(cond), print_operand(destination))
        }
        Instruction::DBcc(counter, address, cond) => format!(
            "db{} {},{}",
            condition(cond),
            register(counter),
            hex(*address as usize)
        ),
        Instruction::BRA(address) => format!("bra {}", hex(*address as usize)),
        Instruction::LINK(frame, displacement) => {
            format!("link {},#${:x}", register(frame), displacement)
        }
        Instruction::UNLK(frame) => format!("unlk {}", register(frame)),
        Instruction::NOT(destination, size) => {
            format!("not.{} {}", size_suffix(*size), print_operand(destination))
        }
        Instruction::OR(source, destination, size) => format!(
            "or.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            print_operand(destination)
        ),
        Instruction::AND(source, destination, size) => format!(
            "and.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            print_operand(destination)
        ),
        Instruction::EOR(source, destination, size) => format!(
            "eor.{} {},{}",
            size_suffix(*size),
            print_operand(source),
            print_operand(destination)
        ),
        Instruction::JSR(target) => format!("jsr {}", print_operand(target)),
        Instruction::ASd(amount, destination, direction, size) => format!(
            "as{}.{} {},{}",
            shift(direction),
            size_suffix(*size),
            print_operand(amount),
            print_operand(destination)
        ),
        Instruction::ROd(amount, destination, direction, size) => format!(
            "ro{}.{} {},{}",
            shift(direction),
            size_suffix(*size),
            print_operand(amount),
            print_operand(destination)
        ),
        Instruction::LSd(amount, destination, direction, size) => format!(
            "ls{}.{} {},{}",
            shift(direction),
            size_suffix(*size),
            print_operand(amount),
            print_operand(destination)
        ),
        Instruction::ROXd(amount, destination, direction, size) => format!(
            "rox{}.{} {},{}",
            shift(direction),
            size_suffix(*size),
            print_operand(amount),
            print_operand(destination)
        ),
        Instruction::BTST(bit, destination) => {
            format!("btst {},{}", print_operand(bit), print_operand(destination))
        }
        Instruction::BCLR(bit, destination) => {
            format!("bclr {},{}", print_operand(bit), print_operand(destination))
        }
        Instruction::BSET(bit, destination) => {
            format!("bset {},{}", print_operand(bit), print_operand(destination))
        }
        Instruction::BCHG(bit, destination) => {
            format!("bchg {},{}", print_operand(bit), print_operand(destination))
        }
        Instruction::JMP(target) => format!("jmp {}", print_operand(target)),
        Instruction::BSR(address) => format!("bsr {}", hex(*address as usize)),
        Instruction::TRAP(vector) => format!("trap #${:x}", vector),
        Instruction::RTS => "rts".to_string(),
        Instruction::NOP => "nop".to_string(),
        Instruction::SIMHALT => "simhalt".to_string(),
        Instruction::MOVEP {
            direction,
            size,
            register: data,
            target,
        } => {
            let (data, target) = (register(data), print_operand(target));
            match direction {
                TargetDirection::ToMemory => {
                    format!("movep.{} {},{}", size_suffix(*size), data, target)
                }
                TargetDirection::FromMemory => {
                    format!("movep.{} {},{}", size_suffix(*size), target, data)
                }
            }
        }
        // The four `move`s and the six immediates that name a half of the
        // status register carry no size: `ccr` is a byte and `sr` a word by
        // definition, so the operand says the width and the fixture does not
        // repeat it.
        Instruction::MOVEtoCCR(source) => format!("move {},ccr", print_operand(source)),
        Instruction::MOVEfromCCR(destination) => {
            format!("move ccr,{}", print_operand(destination))
        }
        Instruction::MOVEtoSR(source) => format!("move {},sr", print_operand(source)),
        Instruction::MOVEfromSR(destination) => format!("move sr,{}", print_operand(destination)),
        Instruction::ANDItoCCR(value) => format!("andi #${:x},ccr", value),
        Instruction::ORItoCCR(value) => format!("ori #${:x},ccr", value),
        Instruction::EORItoCCR(value) => format!("eori #${:x},ccr", value),
        Instruction::ANDItoSR(value) => format!("andi #${:x},sr", value),
        Instruction::ORItoSR(value) => format!("ori #${:x},sr", value),
        Instruction::EORItoSR(value) => format!("eori #${:x},sr", value),
        Instruction::TAS(destination) => format!("tas {}", print_operand(destination)),
        Instruction::RTR => "rtr".to_string(),
        Instruction::CHK(bound, destination) => {
            format!("chk {},{}", print_operand(bound), register(destination))
        }
        Instruction::TRAPV => "trapv".to_string(),
        Instruction::ILLEGAL => "illegal".to_string(),
    }
}

// ---------------------------------------------------------------------------
// The execution fixture
// ---------------------------------------------------------------------------
//
// The interrupt policy below is written out in `tests/corpus/README.md` under
// "Execution fixtures". Its match is exhaustive with no wildcard arm on purpose:
// a new Interrupt has to be given a deterministic answer here rather than stop
// every run that raises it.

/// How many instructions a program may execute before the fixture calls the run
/// `limit`. Most of the corpus is event loops that never end on their own, so
/// most programs stop here rather than terminate.
const RUN_LIMIT: usize = 200_000;

/// The whole of the Interpreter's memory, the 16 MB of a 24-bit address space.
/// `memory_hash_covers_the_whole_address_space` checks that it really is all of it.
const MEMORY_BYTES: usize = 0x0100_0000;

/// One executed program, as the fixture format of `tests/corpus/README.md`.
#[derive(Serialize, PartialEq, Debug)]
struct RunFixture {
    /// `terminated`, `exception` or `limit`.
    status: &'static str,
    /// Instructions executed, the one that raised a runtime error included.
    steps: usize,
    /// `d0` to `d7`, as longs.
    d: Vec<String>,
    /// `a0` to `a7`, as longs; `a7` is the stack pointer.
    a: Vec<String>,
    /// Where the program counter stopped.
    pc: String,
    /// The CCR, as `X:0 N:0 Z:0 V:0 C:0`.
    flags: String,
    /// Everything the display tasks wrote, in order.
    output: String,
    /// FNV-1a 64 of the whole of memory, 16 lowercase hex digits.
    memory: String,
}

/// Runs `source` under the deterministic interrupt policy and builds its
/// execution fixture.
///
/// `name` only names the program in the panic messages.
fn run(name: &str, source: &str, limit: usize) -> RunFixture {
    let mut interpreter = Interpreter::new(
        assemble(name, source),
        Some(InterpreterOptions {
            keep_history: false,
            history_size: 0,
        }),
    );

    let mut steps = 0usize;
    let mut output = String::new();
    let status = loop {
        // A program that ended with the Terminate task leaves that interrupt
        // pending and unanswered, so the end of the run is looked at first.
        if interpreter.has_terminated() {
            break match interpreter.get_status() {
                // Defensive in this version: it reports every exception as an
                // error out of the step below, and never only as a status.
                InterpreterStatus::TerminatedWithException => "exception",
                _ => "terminated",
            };
        }
        if *interpreter.get_status() == InterpreterStatus::Interrupt {
            let interrupt = interpreter
                .get_current_interrupt()
                .unwrap_or_else(|e| panic!("{} reported an interrupt and had none: {:?}", name, e));
            let result = answer(&interrupt, &mut output);
            interpreter.answer_interrupt(result).unwrap_or_else(|e| {
                panic!("{} could not be answered {:?}: {:?}", name, interrupt, e)
            });
            continue;
        }
        // Checked after the interrupt, so an interrupt raised by the last
        // instruction of the budget is still answered before the run stops.
        if steps == limit {
            break "limit";
        }
        // One instruction at a time: `run_with_limit` reports the limit it was
        // given as an error the moment it is reached, and that error, on a limit
        // of one, is how the harness counts a step. It cannot answer `Ok` with a
        // limit of one, but if it ever did, one instruction would still have run.
        match interpreter.run_with_limit(1) {
            Err(RuntimeError::ExecutionLimit(_)) | Ok(_) => steps += 1,
            // The instruction ran and failed. It is counted, and the run stops
            // where the interpreter stopped it.
            Err(_) => {
                steps += 1;
                break "exception";
            }
        }
    };

    let registers = interpreter.get_cpu().get_register_values();
    let flags = interpreter.get_flags_as_array();
    let memory = interpreter
        .get_memory()
        .read_bytes(0, MEMORY_BYTES)
        .unwrap_or_else(|e| panic!("{} could not read its memory back: {:?}", name, e));

    RunFixture {
        status,
        steps,
        d: registers[..8].iter().map(|v| hex(*v as usize)).collect(),
        a: registers[8..].iter().map(|v| hex(*v as usize)).collect(),
        pc: hex(interpreter.get_pc()),
        // `get_flags_as_array` answers in the order carry, overflow, zero,
        // negative, extend; the fixture prints them the way a 68000 lists them.
        flags: format!(
            "X:{} N:{} Z:{} V:{} C:{}",
            flags[4], flags[3], flags[2], flags[1], flags[0]
        ),
        output,
        memory: format!("{:016x}", fnv1a_64(memory)),
    }
}

/// FNV-1a 64, so that the whole of memory reaches the fixture as one short,
/// stable, implementation-independent number.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The deterministic interrupt policy: every display task appends to `output`,
/// every input task answers the same thing every time, and everything else is
/// acknowledged with no effect.
///
/// `tests/corpus/README.md`, "Execution fixtures", is the specification.
fn answer(interrupt: &Interrupt, output: &mut String) -> InterruptResult {
    match interrupt {
        // Display tasks
        Interrupt::DisplayStringWithCRLF(string) => {
            output.push_str(string);
            output.push('\n');
            InterruptResult::DisplayStringWithCRLF
        }
        Interrupt::DisplayStringWithoutCRLF(string) => {
            output.push_str(string);
            InterruptResult::DisplayStringWithoutCRLF
        }
        Interrupt::DisplayNumber(number) => {
            output.push_str(&number.to_string());
            InterruptResult::DisplayNumber
        }
        Interrupt::DisplayNumberInBase { value, base } => {
            output.push_str(&print_in_base(*value, *base));
            InterruptResult::DisplayNumberInBase
        }
        Interrupt::DisplayChar(character) => {
            output.push(*character);
            InterruptResult::DisplayChar
        }
        Interrupt::DisplaySignedNumberInField { value, width } => {
            let number = value.to_string();
            for _ in number.chars().count()..*width as usize {
                output.push(' ');
            }
            output.push_str(&number);
            InterruptResult::DisplaySignedNumberInField
        }
        Interrupt::DisplayStringAndNumber { string, number } => {
            output.push_str(string);
            output.push_str(&number.to_string());
            InterruptResult::DisplayStringAndNumber
        }
        Interrupt::DisplayStringAndReadNumber(string) => {
            output.push_str(string);
            InterruptResult::DisplayStringAndReadNumber(7)
        }

        // Input tasks: one answer each, the same one every run
        Interrupt::ReadNumber => InterruptResult::ReadNumber(7),
        Interrupt::ReadChar => InterruptResult::ReadChar('a'),
        Interrupt::ReadKeyboardString => InterruptResult::ReadKeyboardString("test".to_string()),
        Interrupt::GetTime => InterruptResult::GetTime(0),
        Interrupt::CheckKeyboardInput => InterruptResult::CheckKeyboardInput(false),
        Interrupt::GetKeyState(request) => InterruptResult::GetKeyState(match request {
            KeyStateRequest::Keys(_) => KeyStateResult::Keys([false; 4]),
            KeyStateRequest::LastKeys => KeyStateResult::LastKeys { up: 0, down: 0 },
        }),
        Interrupt::ReadMouse(_) => InterruptResult::ReadMouse {
            flags: 0,
            x: 0,
            y: 0,
        },
        Interrupt::GetPixelColor(_, _) => InterruptResult::GetPixelColor(0),
        Interrupt::GetPenPosition => InterruptResult::GetPenPosition(0, 0),
        Interrupt::GetScreenSize => InterruptResult::GetScreenSize(640, 480),
        Interrupt::GetTextCursorPosition => InterruptResult::GetTextCursorPosition(0, 0),

        // Everything else: acknowledged, no effect
        Interrupt::Terminate => InterruptResult::Terminate,
        Interrupt::Delay(_) => InterruptResult::Delay,
        Interrupt::SetSimulatorShortcuts(_) => InterruptResult::SetSimulatorShortcuts,
        Interrupt::SetPenColor(_) => InterruptResult::SetPenColor,
        Interrupt::SetFillColor(_) => InterruptResult::SetFillColor,
        Interrupt::DrawPixel(_, _) => InterruptResult::DrawPixel,
        Interrupt::DrawLine(_, _, _, _) => InterruptResult::DrawLine,
        Interrupt::DrawLineTo(_, _) => InterruptResult::DrawLineTo,
        Interrupt::MoveTo(_, _) => InterruptResult::MoveTo,
        Interrupt::DrawRectangle(_, _, _, _) => InterruptResult::DrawRectangle,
        Interrupt::DrawEllipse(_, _, _, _) => InterruptResult::DrawEllipse,
        Interrupt::FloodFill(_, _) => InterruptResult::FloodFill,
        Interrupt::DrawUnfilledRectangle(_, _, _, _) => InterruptResult::DrawUnfilledRectangle,
        Interrupt::DrawUnfilledEllipse(_, _, _, _) => InterruptResult::DrawUnfilledEllipse,
        Interrupt::SetDrawingMode(_) => InterruptResult::SetDrawingMode,
        Interrupt::SetPenWidth(_) => InterruptResult::SetPenWidth,
        Interrupt::Repaint => InterruptResult::Repaint,
        Interrupt::DrawText(_, _, _) => InterruptResult::DrawText,
        Interrupt::SetScreenSize(_, _) => InterruptResult::SetScreenSize,
        Interrupt::SetScreenMode(_) => InterruptResult::SetScreenMode,
        Interrupt::ClearScreen => InterruptResult::ClearScreen,
        Interrupt::SetTextCursorPosition(_, _) => InterruptResult::SetTextCursorPosition,
    }
}

/// An unsigned number in a base of 2 to 36, digits `0` to `9` then `a` to `z`.
///
/// The Interpreter refuses a base outside that range before the task ever
/// reaches the policy; the range is clamped here so that no answer can loop.
fn print_in_base(value: u32, base: u8) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let base = (base as u32).clamp(2, 36);
    if value == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    let mut rest = value;
    while rest > 0 {
        digits.push(DIGITS[(rest % base) as usize] as char);
        rest /= base;
    }
    digits.iter().rev().collect()
}

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

fn corpus_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join(name)
}

fn snapshots_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("corpus")
        .join("snapshots")
}

/// Every file of `directory` with one of `extensions`, by file name, so that the
/// order does not depend on the file system.
fn corpus_files(directory: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", directory.display(), e))
        .map(|entry| entry.expect("a readable directory entry").path())
        .filter(|path| match path.extension().and_then(|e| e.to_str()) {
            Some(extension) => extensions.contains(&extension.to_lowercase().as_str()),
            None => false,
        })
        .collect();
    files.sort();
    files
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .expect("a corpus file name")
        .to_string()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e))
}

fn settings() -> insta::Settings {
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(snapshots_dir());
    settings.set_prepend_module_to_snapshot(false);
    settings
}

/// The 30 asm-editor programs all assemble, and this is what they assemble to.
#[test]
fn editor_programs() {
    let directory = corpus_dir("editor");
    let files = corpus_files(&directory, &["asm", "x68"]);
    assert_eq!(
        files.len(),
        30,
        "the corpus should hold the 30 asm-editor programs, found {} in {}",
        files.len(),
        directory.display()
    );
    let mut names: Vec<String> = files.iter().map(|path| stem(path)).collect();
    names.sort();
    let unique = names.len();
    names.dedup();
    assert_eq!(
        names.len(),
        unique,
        "two corpus programs share a file stem and would share a snapshot"
    );

    settings().bind(|| {
        for path in &files {
            let name = stem(path);
            let fixture = dump(&name, &read(path));
            insta::assert_json_snapshot!(name, fixture);
        }
    });
}

/// The 3 EASy68K originals do not assemble; these are the Diagnostics they
/// raise, so that the rewrite's effect on them shows up as a diff.
///
/// The design record's "Tests" item 2. On 1.4.2 these fixtures were 88, 138 and
/// 64 error strings, nearly all of them the old checker misreading a Label in
/// column 1; what is left is the features s68k does not implement, each named
/// by its own message. `tests/corpus/README.md`, "Diagnostics fixtures", is the
/// format, and `the_easy68k_originals_raise_only_what_is_not_implemented` in
/// `src/test/diagnostics.rs` is the same finding counted by code.
#[test]
fn easy68k_programs_do_not_assemble() {
    let directory = corpus_dir("easy68k");
    let files = corpus_files(&directory, &["x68"]);
    assert_eq!(
        files.len(),
        3,
        "the corpus should hold the 3 EASy68K originals, found {} in {}",
        files.len(),
        directory.display()
    );

    settings().bind(|| {
        for path in &files {
            let name = stem(path);
            let file = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a file name");
            let mut files = Files::new();
            files.insert_text(file, read(path));
            let assembly = crate::assembler::assemble(&files, file);
            assert!(
                assembly.program.is_none(),
                "{} now assembles; the fixture and tests/corpus/README.md have \
                 to be updated on purpose",
                name
            );
            let diagnostics: Vec<&Diagnostic> = assembly.diagnostics.iter().collect();
            insta::assert_json_snapshot!(format!("{}-errors", name), diagnostics);
        }
    });
}

/// The printer rules the corpus does not exercise, spelled out.
///
/// `tests/corpus/README.md` documents these; a change here is a change there.
#[test]
fn printer_rules() {
    let source = "\
regs reg d0-d2/a0/a6
    movem.l d0-d2/a0/a6,-(sp)
    movem.l (sp)+,d0-d2/a0/a6
    movem.w d3,(a0)
    movem.w (a0),d3
    cmpm.b (a0)+,(a1)+
    exg d0,a1
    pea $2000
    ext.w d0
    ext.l d0
    not.b d0
    btst #3,d0
    bset #3,d0
    bclr #3,d0
    bchg #3,d0
    shs d0
    slo d0
    st d0
    lsl (a0)
    asr (a0)
    divs #2,d0
    muls #2,d0
    moveq #7,d0
    link a6,#4
    unlk a6
    jmp $1000
    nop
    move.l sp,a5
    move.l -8(a6),d0
    move.l 0(a6),d0
    move.l 4(a6,d1.w),d0
    move.l 4(a6,a2.l),d0
loop:
    dbra d7,loop
    dbhs d7,loop
    bhs loop
    blo loop
    bra loop
    bsr loop
    add #1,d0
    move #1,a0
    cmp #1,d0
    rts
    add #1,a0
    sub #1,a0
    cmp #1,a0
    jsr $1000
    ori.b #1,d0
    eori.l #1,d0
    suba.l a1,a0
    sub.l a1,a0
    add.l a1,a0
    rol.w #1,d0
    ror.l #1,d0
    bvc loop
    bvs loop
    bmi loop
    moveq #-1,d0
    move.l #-1,d1
    movem.l regs,-(sp)
    movem.l (sp)+,regs
    simhalt
    movep.w d0,4(a1)
    movep.l 4(a1),d0
    move.w d0,ccr
    move.w #$1f,ccr
    move.w ccr,d0
    move.w d0,sr
    move.w sr,d0
    andi.b #$1f,ccr
    ori.b #$1,ccr
    eori.b #$4,ccr
    andi.w #$00,sr
    ori.w #$700,sr
    eori.w #$2000,sr
    tas (a0)
    tas d0
    rtr
    chk #$a,d0
    chk (a0),d1
    trapv
    illegal
    addx.l d0,d1
    addx.b -(a0),-(a1)
    subx.w d0,d1
    negx.l (a0)
    abcd d0,d1
    abcd -(a0),-(a1)
    sbcd.b d0,d1
    nbcd (a0)
    roxl.b #1,d0
    roxr.l d1,d0
    roxl (a0)
    move.l here(pc),d0
    lea here(pc),a0
    move.l here(pc,d1.w),d1
    move.l (pc,d1.w),d2
    move.l $1000.w,d3
    move.l $1000.l,d4
here: dc.l 1
    move.l here(pc),d5
";
    let expected = [
        "movem.l d0-d2/a0/a6,-(a7)",
        "movem.l (a7)+,d0-d2/a0/a6",
        "movem.w d3,(a0)",
        "movem.w (a0),d3",
        "cmpm.b (a0)+,(a1)+",
        "exg d0,a1",
        "pea $2000",
        "ext.w d0",
        "ext.l d0",
        "not.b d0",
        "btst #$3,d0",
        "bset #$3,d0",
        "bclr #$3,d0",
        "bchg #$3,d0",
        "scc d0",
        "scs d0",
        "st d0",
        "lsl.w #$1,(a0)",
        "asr.w #$1,(a0)",
        "divs #$2,d0",
        "muls #$2,d0",
        "moveq #$7,d0",
        "link a6,#$4",
        "unlk a6",
        "jmp $1000",
        "nop",
        "movea.l a7,a5",
        "move.l -8(a6),d0",
        "move.l 0(a6),d0",
        "move.l 4(a6,d1.w),d0",
        "move.l 4(a6,a2.l),d0",
        "dbf d7,$107c",
        "dbcc d7,$107c",
        "bcc $107c",
        "bcs $107c",
        "bra $107c",
        "bsr $107c",
        "addi.w #$1,d0",
        "movea.w #$1,a0",
        "cmpi.w #$1,d0",
        "rts",
        // `add`/`sub` rewrite an immediate before they look at the destination,
        // `cmp` looks at the destination first; the fixtures show the difference.
        "addi.w #$1,a0",
        "subi.w #$1,a0",
        "cmpa.w #$1,a0",
        "jsr $1000",
        "ori.b #$1,d0",
        "eori.l #$1,d0",
        "suba.l a1,a0",
        "suba.l a1,a0",
        "adda.l a1,a0",
        "rol.w #$1,d0",
        "ror.l #$1,d0",
        "bvc $107c",
        "bvs $107c",
        "bmi $107c",
        // The count of a quick form is not an operand: this version keeps it
        // in eight bits, so the same -1 truncates where a 32-bit immediate
        // operand sign extends. `addq`, `subq` and `trap` truncate the same way.
        "moveq #$ff,d0",
        "move.l #$ffffffff,d1",
        // A `reg` symbol is lowered exactly as the list it stands for, in both
        // directions, so these two read as the first two lines of the source.
        "movem.l d0-d2/a0/a6,-(a7)",
        "movem.l (a7)+,d0-d2/a0/a6",
        "simhalt",
        // Phase 3's first group. `movep` carries its size, because a word and
        // a long are a real choice; the four `move`s and the six immediates
        // that name a half of the status register carry none, because `ccr` is
        // a byte and `sr` a word by definition.
        "movep.w d0,4(a1)",
        "movep.l 4(a1),d0",
        "move d0,ccr",
        "move #$1f,ccr",
        "move ccr,d0",
        "move d0,sr",
        "move sr,d0",
        "andi #$1f,ccr",
        "ori #$1,ccr",
        "eori #$4,ccr",
        "andi #$0,sr",
        "ori #$700,sr",
        "eori #$2000,sr",
        "tas (a0)",
        "tas d0",
        "rtr",
        "chk #$a,d0",
        "chk (a0),d1",
        "trapv",
        "illegal",
        // Phase 3's second group. `addx`, `subx`, `negx` and the rotates carry
        // their size, because all three are a real choice; the three decimal
        // instructions carry none, because a byte is the only size they have,
        // the way `tas` has only a byte.
        "addx.l d0,d1",
        "addx.b -(a0),-(a1)",
        "subx.w d0,d1",
        "negx.l (a0)",
        "abcd d0,d1",
        "abcd -(a0),-(a1)",
        "sbcd d0,d1",
        "nbcd (a0)",
        "roxl.b #$1,d0",
        "roxr.l d1,d0",
        "roxl.w #$1,(a0)",
        // Phase 3's last group. A PC-relative operand is written as the
        // address it reaches and printed as the displacement the Assembler
        // stored: `here` is 22 bytes past the extension word of the first of
        // these lines, 18 past the second's, and 6 bytes *behind* the last
        // one's. `(pc,d1.w)` writes no address, so its displacement is zero.
        "move.l 22(pc),d0",
        "lea 18(pc),a0",
        "move.l 14(pc,d1.w),d1",
        "move.l 0(pc,d1.w),d2",
        // A forced width is not printed: `.w` and `.l` name the same address
        // here and the Program holds the address alone.
        "move.l $1000,d3",
        "move.l $1000,d4",
        "move.l -6(pc),d5",
    ];
    let fixture = dump("printer_rules", source);
    let texts: Vec<&str> = fixture
        .instructions
        .iter()
        .map(|i| i.text.as_str())
        .collect();
    assert_eq!(texts, expected);
}

/// The rules for what the pipeline of this version cannot reach, printed straight
/// from the instruction.
///
/// `extb` is one of them: the semantic checker of this version does not know the
/// mnemonic, although the compiler does, so no source can produce the byte to
/// long `ext`. The empty `movem` list is another: no source writes one, but the
/// printer has to say something.
#[test]
fn printer_rules_out_of_reach() {
    assert_eq!(
        print_instruction(&Instruction::EXT(
            RegisterOperand::Data(0),
            Size::Byte,
            Size::Long
        )),
        "extb.l d0"
    );
    // The five size pairs `extb` and `ext` cannot produce still have to print
    // something; the rule is "the destination size", the same as the reachable ones.
    assert_eq!(
        print_instruction(&Instruction::EXT(
            RegisterOperand::Data(0),
            Size::Long,
            Size::Word
        )),
        "ext.w d0"
    );
    assert_eq!(
        print_instruction(&Instruction::EXT(
            RegisterOperand::Data(0),
            Size::Word,
            Size::Byte
        )),
        "ext.b d0"
    );
    assert_eq!(
        print_instruction(&Instruction::Bcc(0x1000, Condition::True)),
        "bt $1000"
    );
    assert_eq!(
        print_instruction(&Instruction::DBcc(
            RegisterOperand::Data(7),
            0x1000,
            Condition::LowOrSame
        )),
        "dbls d7,$1000"
    );
    // The register list, on its own: every group form, and the empty mask.
    assert_eq!(print_register_list(0b0000_0000_0000_0000), "");
    assert_eq!(print_register_list(0b0000_0001_0000_0001), "d0/a0");
    assert_eq!(print_register_list(0b1111_1111_1111_1111), "d0-d7/a0-a7");
    // d7 and a0 are neighbours in the mask but never make one range.
    assert_eq!(print_register_list(0b0000_0001_1000_0000), "d7/a0");
    assert_eq!(print_register_list(0b1000_0000_0000_0011), "d0-d1/a7");
}

/// `Directives/section.htm`'s own example, as a fixture.
///
/// Sixteen location counters, each going on from where it was left: the data of
/// section 1 is one run in the fixture's `memory` and not two, because `msg2`
/// carries on from where `msg1` stopped while the code of section 0 was being
/// laid out in between. Every address below is worked out by hand from the help
/// and from the default origin, and none of it is read back from the Assembler.
#[test]
fn the_section_example_of_the_help() {
    // The help's `<code>` is two instructions here, so that section 0 has
    // something to lay out; everything else is its own.
    let source = "\
CODE    EQU     0
DATA    EQU     1
        SECTION DATA
        ORG     $2000
msg1    DC.B    'Hello World',$d,$a,0
        SECTION CODE
        ORG     $1000
        MOVE.L  #1,D0
        NOP
        SECTION DATA
msg2    DC.B    'EASy68K Rules!',$d,$a,0
";
    let fixture = dump("section_example", source);
    // `msg1` is 'Hello World' (11) + $d + $a + 0 = 14 bytes from $2000, so
    // section 1 has reached $200e when the program comes back to it, and `msg2`
    // is 'EASy68K Rules!' (14) + 3 = 17 bytes from there.
    let memory: Vec<(&str, usize)> = fixture
        .memory
        .iter()
        .map(|run| match run {
            FixtureMemory::Bytes { address, bytes } => (address.as_str(), bytes.len() / 2),
            FixtureMemory::Reserved { address, reserved } => (address.as_str(), *reserved),
        })
        .collect();
    assert_eq!(memory, vec![("$2000", 14), ("$200e", 17)]);
    // Section 0 starts at the default origin, which is where its `ORG` puts it
    // anyway; the two instructions are four bytes each.
    let instructions: Vec<(&str, &str)> = fixture
        .instructions
        .iter()
        .map(|instruction| (instruction.address.as_str(), instruction.text.as_str()))
        .collect();
    assert_eq!(
        instructions,
        vec![("$1000", "move.l #$1,d0"), ("$1004", "nop")]
    );
    // The two Labels are Labels, with the addresses above.
    assert_eq!(fixture.labels["msg1"].address, "$2000");
    assert_eq!(fixture.labels["msg2"].address, "$200e");
    // `CODE` and `DATA` are Constants and no more in the fixture than any other
    // `equ` is.
    assert_eq!(fixture.labels.len(), 2);
    // The program starts at its first instruction, which is in section 0.
    assert_eq!(fixture.entry, "$1000");
}

/// `Directives/offset.htm`'s stack-frame example, as a fixture.
///
/// The whole of an `offset` region is names: nothing is placed, so the fixture's
/// `memory` is empty and its `labels` hold none of the three fields — they are
/// Constants, because an offset into a stack frame is a value and there is no
/// line of the program at it. What the region did is visible in the
/// instructions, which carry the offsets as their displacements.
#[test]
fn the_offset_stack_frame_example_of_the_help() {
    let source = "\
SIZE    EQU -3*4
        OFFSET  SIZE
num1    DS.L    1
num2    DS.L    1
num3    DS.L    1
        ORG     *
        LINK    A0,#SIZE
        MOVEM.L A0-A1,-(A7)
        MOVE.L  #$11111111,(num1,A0)
        MOVE.L  #$22222222,(num2,A0)
        MOVE.L  #$33333333,(num3,A0)
";
    let fixture = dump("offset_example", source);
    // Three long words counting up from -12: -12, -8, -4. `ORG *` comes back to
    // the address the region shadowed, which is the default origin, and the
    // five instructions are four bytes each from there.
    let instructions: Vec<(&str, &str)> = fixture
        .instructions
        .iter()
        .map(|instruction| (instruction.address.as_str(), instruction.text.as_str()))
        .collect();
    assert_eq!(
        instructions,
        vec![
            ("$1000", "link a0,#$fffffff4"),
            ("$1004", "movem.l a0-a1,-(a7)"),
            ("$1008", "move.l #$11111111,-12(a0)"),
            ("$100c", "move.l #$22222222,-8(a0)"),
            ("$1010", "move.l #$33333333,-4(a0)"),
        ]
    );
    assert!(
        fixture.memory.is_empty(),
        "an offset region places nothing: {:?}",
        fixture.memory.len()
    );
    assert!(
        fixture.labels.is_empty(),
        "the three fields are Constants and not Labels: {:?}",
        fixture.labels.keys().collect::<Vec<_>>()
    );
    // The symbol listing is where they are, and it says what they are.
    let program = assemble("offset_example", source);
    for (name, value) in [("num1", -12), ("num2", -8), ("num3", -4)] {
        let symbol = &program.symbols()[name];
        assert_eq!(symbol.value, value, "{name}");
        assert_eq!(symbol.kind, SymbolKind::Constant, "{name}");
    }
    assert_eq!(fixture.entry, "$1000");
}

/// The Entry point: `end`'s operand, else a Label named `START` whatever its
/// case, else the first instruction.
///
/// No fixture can tell the three apart — `entry` is the first instruction's
/// address in all 30 of them, `bad-apple.x68` reaching it through `START:`, the
/// five `.x68` programs that write `start:` reaching the same address through
/// the label and the first instruction alike, and no corpus program writing
/// `end` — so a regression in the lookup would change no snapshot. This is what
/// holds it instead. `tests/corpus/README.md` records that the fixtures cannot
/// see it.
#[test]
fn entry_point_is_a_start_label_of_either_case_then_the_first_instruction() {
    // A Label named `START` wins over the first instruction.
    let uppercase = "\
    ORG $1000
first:
    nop
START:
    nop
";
    assert_eq!(dump("entry_uppercase", uppercase).entry, "$1004");
    // The fallback name is s68k's own convention and not a word the program
    // wrote, so it is matched whatever its case: the lowercase `start:` that
    // five of the six `.x68` programs write is the entry point too.
    let lowercase = "\
    ORG $1000
first:
    nop
start:
    nop
";
    assert_eq!(dump("entry_lowercase", lowercase).entry, "$1004");
    // `end` beats both, and it is the source 1.4.2 could not even parse.
    let ended = "\
    ORG $1000
first:
    nop
START:
    nop
last:
    rts
    END last
";
    assert_eq!(dump("entry_end", ended).entry, "$1008");
}

/// The 30 asm-editor programs all run, and this is where they get to.
#[test]
fn editor_programs_run() {
    let directory = corpus_dir("editor");
    let files = corpus_files(&directory, &["asm", "x68"]);
    assert_eq!(
        files.len(),
        30,
        "the corpus should hold the 30 asm-editor programs, found {} in {}",
        files.len(),
        directory.display()
    );

    settings().bind(|| {
        for path in &files {
            let name = stem(path);
            let fixture = run(&name, &read(path), RUN_LIMIT);
            insta::assert_json_snapshot!(format!("{}-run", name), fixture);
        }
    });
}

/// The interrupt policy, spelled out: every answer it gives and every way it
/// prints, in one program that displays what it was answered.
///
/// The corpus reaches only a few of these tasks, so this is what holds the
/// policy of `tests/corpus/README.md` in place. It runs twice and the two runs
/// have to be identical, which is the determinism the fixtures rest on.
#[test]
fn run_policy() {
    let source = "\
    ORG $1000
line: dc.b 'hi',0
name: dc.b 'n=',0
ask: dc.b '?',0
buffer: ds.b 8
start:
    move.l #line,a1
    move.b #13,d0       ; string with CRLF
    trap #15
    move.l #line,a1
    move.b #14,d0       ; string without CRLF
    trap #15
    move.l #-5,d1
    move.b #3,d0        ; number
    trap #15
    move.b #'z',d1
    move.b #6,d0        ; char
    trap #15
    move.l #255,d1
    move.b #16,d2
    move.b #15,d0       ; number in base 16
    trap #15
    move.l #-5,d1
    move.b #6,d2
    move.b #20,d0       ; signed number in a field of 6
    trap #15
    move.l #name,a1
    move.l #42,d1
    move.b #17,d0       ; string and number
    trap #15
    move.l #ask,a1
    move.b #18,d0       ; string and number read
    trap #15
    move.b #3,d0
    trap #15
    move.b #4,d0        ; number read
    trap #15
    move.b #3,d0
    trap #15
    move.b #5,d0        ; char read
    trap #15
    move.b #6,d0
    trap #15
    move.l #$FFFFFFFF,d1
    move.b #8,d0        ; time
    trap #15
    move.b #3,d0
    trap #15
    move.l #$FFFFFF00,d1
    move.b #7,d0        ; pending keyboard input
    trap #15
    and.l #$FF,d1
    move.b #3,d0
    trap #15
    move.l #0,d1
    move.b #19,d0       ; last keys
    trap #15
    move.b #3,d0
    trap #15
    move.l #0,d1
    move.b #61,d0       ; mouse
    trap #15
    move.b #3,d0
    trap #15
    move.l #0,d1
    move.b #33,d0       ; screen size
    trap #15
    move.b #3,d0
    trap #15
    move.l #$FF,d1
    move.b #11,d0       ; text cursor position
    trap #15
    move.b #3,d0
    trap #15
    move.l #0,d1
    move.l #0,d2
    move.b #83,d0       ; pixel colour
    trap #15
    move.l d0,d1
    move.b #3,d0
    trap #15
    move.l #$FFFFFFFF,d1
    move.l #$FFFFFFFF,d2
    move.b #96,d0       ; pen position
    trap #15
    and.l #$FFFF,d1
    move.b #3,d0
    trap #15
    move.l #1000,d1
    move.b #23,d0       ; delay
    trap #15
    move.l #buffer,a1
    move.l #0,d1
    move.b #2,d0        ; keyboard string
    trap #15
    move.b #3,d0
    trap #15
    move.l #buffer,a1
    move.b #14,d0
    trap #15
    move.b #9,d0        ; terminate
    trap #15
";
    let expected = concat!(
        "hi\n",     // 13, the string and a newline
        "hi",       // 14, the string alone
        "-5",       // 3, the signed number in D1.L
        "z",        // 6, the character in D1.B
        "ff",       // 15, D1.L in the base in D2.B
        "    -5",   // 20, right justified in a field of D2.B columns
        "n=42",     // 17, the string then the number
        "?",        // 18, the string it displays
        "7",        // and the number task after it shows the 7 it answered
        "7",        // 4, the answer 7
        "a",        // 5, the answer 'a'
        "0",        // 8, the answer 0
        "0",        // 7, the answer "no input pending"
        "0",        // 19, the answer "no last keys"
        "0",        // 61, the answer "no buttons, at 0,0"
        "41943520", // 33, ($280 << 16) | $1E0, the answer 640 by 480
        "0",        // 11 with $00FF, the answer "column 0, row 0"
        "0",        // 83, the answer "colour 0"
        "0",        // 96, the answer "pen at 0,0"
        "4",        // 2, the length of "test"
        "test",
    );

    let fixture = run("run_policy", source, RUN_LIMIT);
    assert_eq!(fixture.output, expected);
    assert_eq!(fixture.status, "terminated");
    // The same program, run again, has to give the same fixture back.
    assert_eq!(fixture, run("run_policy", source, RUN_LIMIT));
}

/// A runtime error ends the run as `exception`, and `steps` counts the
/// instruction that raised it.
///
/// Every one of the 30 programs either terminates or reaches the limit, so
/// without this the `exception` status of `tests/corpus/README.md` and the
/// counting rule beside it would be recorded by no test at all.
#[test]
fn a_runtime_error_ends_the_run_with_an_exception() {
    // Division by zero: the second instruction raises it and is counted.
    let divide_by_zero = "\
    ORG $1000
    move.l #10,d0
    divu #0,d0
    move.b #9,d0
    trap #15
";
    let fixture = run("divide_by_zero", divide_by_zero, RUN_LIMIT);
    assert_eq!(fixture.status, "exception");
    assert_eq!(fixture.steps, 2);

    // An address with no instruction on it, below the last instruction of the
    // program: the jump runs, then the step that lands there fails and counts.
    let into_a_gap = "\
    ORG $1000
    jmp $1004
    ORG $2000
    nop
";
    let fixture = run("into_a_gap", into_a_gap, RUN_LIMIT);
    assert_eq!(fixture.status, "exception");
    assert_eq!(fixture.steps, 2);
}

/// The memory the fixture hashes is the whole address space and not a part of it.
#[test]
fn memory_hash_covers_the_whole_address_space() {
    let interpreter = Interpreter::new(
        assemble("memory", "    nop\n"),
        Some(InterpreterOptions {
            keep_history: false,
            history_size: 0,
        }),
    );
    let memory = interpreter.get_memory();
    assert!(
        memory.read_bytes(0, MEMORY_BYTES).is_ok(),
        "memory is smaller than the {} bytes the fixture hashes",
        MEMORY_BYTES
    );
    assert!(
        memory.read_bytes(0, MEMORY_BYTES + 1).is_err(),
        "memory is larger than the {} bytes the fixture hashes",
        MEMORY_BYTES
    );
}

/// `ds` reserves memory and writes nothing to it, and memory starts as `$ff`,
/// so a `ds` block reads back as `$ff` throughout.
///
/// This is the running half of the `ds` fix of `tests/corpus/README.md`: the
/// assembly fixtures record it as `reserved` where 1.4.2 wrote a `zeroed`
/// count of an eighth of the block, and the memory hash of every execution
/// fixture of a program with a `ds` in it carries the same change. On 1.4.2
/// this program printed `0,255`.
#[test]
fn ds_reserves_memory_without_writing_it() {
    let source = "\
    ORG $1000
start:
    move.l #buf,a0
    move.l #0,d1
    move.b (a0),d1
    move.b #3,d0
    trap #15
    move.b #',',d1
    move.b #6,d0
    trap #15
    move.l #0,d1
    move.b 7(a0),d1
    move.b #3,d0
    trap #15
    move.b #9,d0
    trap #15
    ORG $2000
buf: ds.b 8
";
    // Neither the first byte of the eight nor the eighth is written.
    assert_eq!(run("ds", source, RUN_LIMIT).output, "255,255");
}
