//! What a write reports: the value it replaced beside the value it wrote.
//!
//! One test per rule of the contract. Every write an Execution step records
//! carries both sides, read where the write happened — a register write reports
//! the whole register, a memory write reports the value at the width it was
//! made — so that nothing about what a step did is reconstructed afterwards by
//! its reader.

use serde_json::Value;

use crate::assembler::program::Program;
use crate::debugger::{ExecutionStep, MutationOperation};
use crate::instructions::{RegisterOperand, Size};
use crate::interpreter::{Interpreter, InterpreterOptions, InterpreterStatus};

/// The Program `code` assembles to, or the Diagnostics that stopped it.
fn assemble(code: &str) -> Program {
    let assembly = crate::assembler::assemble_source(code);
    match assembly.program {
        Some(program) => program,
        None => panic!(
            "code did not assemble: {:?}",
            assembly
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message())
                .collect::<Vec<String>>()
        ),
    }
}

/// An Interpreter over `code` that keeps a history, which is what carries the
/// mutations.
fn with_history(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: true,
            history_size: 100,
        }),
    )
}

/// An Interpreter over `code` that keeps none, which records no mutation at all.
fn without_history(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: false,
            history_size: 0,
        }),
    )
}

fn step(interpreter: &mut Interpreter, what: &str) {
    interpreter
        .step()
        .unwrap_or_else(|error| panic!("{} should run: {:?}", what, error));
}

/// The mutations of the newest step, in the order they were made.
fn last_mutations(interpreter: &Interpreter) -> Vec<MutationOperation> {
    interpreter.get_last_steps(1)[0].get_mutations().clone()
}

fn as_json(step: &ExecutionStep) -> Value {
    serde_json::to_value(step).expect("a step serialises")
}

/// The write mutations of a step as JSON, which is the shape the editor reads.
fn write_mutations_json(step: &ExecutionStep) -> Vec<Value> {
    as_json(step)["mutations"]
        .as_array()
        .expect("a mutations list")
        .iter()
        .filter(|mutation| {
            matches!(
                mutation["type"].as_str(),
                Some("WriteRegister") | Some("WriteMemory") | Some("WriteMemoryBytes")
            )
        })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// 1. What each kind of write reports
// ---------------------------------------------------------------------------

#[test]
fn a_sized_register_write_reports_the_whole_register_on_both_sides() {
    // `move.b` changes the low byte only, and the report is the whole register
    // both before and after it, which is what the panels draw.
    let mut interpreter = with_history(
        "    org $1000
    move.l #$aabbccdd,d0
    move.b #$01,d0
",
    );
    step(&mut interpreter, "the long move");
    step(&mut interpreter, "the byte move");
    match last_mutations(&interpreter).as_slice() {
        [MutationOperation::WriteRegister {
            register,
            old,
            new,
            size,
        }] => {
            assert_eq!(*register, RegisterOperand::Data(0));
            assert_eq!(*size, Size::Byte, "the write was made at a byte");
            assert_eq!(*old, 0xaabbccdd, "the whole register before it");
            assert_eq!(*new, 0xaabbcc01, "and the whole register after it");
        }
        other => panic!("expected one register write, got {:?}", other),
    }
    assert_eq!(
        interpreter.get_register_value(RegisterOperand::Data(0), Size::Long),
        0xaabbcc01,
        "which is what the register holds"
    );
}

#[test]
fn an_address_register_written_by_an_addressing_mode_reports_both_sides() {
    // pre-decrement writes the address register through `set_a_reg_sized`,
    // which is a write of its own and reports the same two values.
    let mut interpreter = with_history(
        "    org $1000
    movea.l #$2004,a0
    move.l #$11223344,-(a0)
",
    );
    step(&mut interpreter, "the movea");
    step(&mut interpreter, "the move to -(a0)");
    match last_mutations(&interpreter).as_slice() {
        [MutationOperation::WriteRegister {
            register,
            old,
            new,
            size: _,
        }, MutationOperation::WriteMemory { .. }] => {
            assert_eq!(*register, RegisterOperand::Address(0));
            assert_eq!(*old, 0x2004, "the pointer before the decrement");
            assert_eq!(*new, 0x2000, "and after it");
        }
        other => panic!(
            "expected a register write then a memory write, got {:?}",
            other
        ),
    }
}

#[test]
fn a_memory_write_reports_the_value_it_replaced_and_the_value_it_left() {
    // memory starts as $ff, so an untouched word reads back as $ffff.
    let mut interpreter = with_history(
        "    org $1000
    move.w #$1234,$2000
",
    );
    step(&mut interpreter, "the word move");
    match last_mutations(&interpreter).as_slice() {
        [MutationOperation::WriteMemory {
            address,
            old,
            new,
            size,
        }] => {
            assert_eq!(*address, 0x2000);
            assert_eq!(*size, Size::Word, "the width the write was made at");
            assert_eq!(*old, 0xffff, "the two bytes it replaced");
            assert_eq!(*new, 0x1234, "and the two bytes it left");
        }
        other => panic!("expected one memory write, got {:?}", other),
    }
    assert_eq!(
        interpreter.get_memory().read_bytes(0x2000, 3).unwrap(),
        &[0x12, 0x34, 0xff],
        "and nothing past the width it was made at"
    );
}

#[test]
fn a_memory_write_reports_what_the_store_kept_of_the_value() {
    // `write_size` keeps the low bytes of the value, so the reported new value
    // is what is there and not the argument that was passed.
    let mut interpreter = with_history(
        "    org $1000
    nop
",
    );
    step(&mut interpreter, "the nop");
    interpreter
        .set_memory_value(0x3000, Size::Word, 0x11223344)
        .expect("the word store");
    match last_mutations(&interpreter).as_slice() {
        [MutationOperation::WriteMemory { old, new, .. }] => {
            assert_eq!(*old, 0xffff);
            assert_eq!(
                *new, 0x3344,
                "the low word, which is what the store put there"
            );
        }
        other => panic!("expected one memory write, got {:?}", other),
    }
    assert_eq!(
        interpreter
            .get_memory()
            .read_size(0x3000, Size::Word)
            .unwrap(),
        0x3344
    );
}

#[test]
fn a_run_of_bytes_reports_the_bytes_it_replaced_and_the_bytes_it_left() {
    // the host's answer to a "read string" trap writes bytes on the
    // instruction's behalf, which is the only run of bytes an instruction
    // writes.
    let mut interpreter = with_history(
        "    org $1000
    movea.l #$2000,a1
    move.b #2,d0
    trap #15
",
    );
    step(&mut interpreter, "the movea");
    step(&mut interpreter, "the move");
    step(&mut interpreter, "the trap");
    assert_eq!(*interpreter.get_status(), InterpreterStatus::Interrupt);
    interpreter
        .answer_interrupt(crate::instructions::InterruptResult::ReadKeyboardString(
            "hi".to_string(),
        ))
        .expect("the answer");
    let bytes = last_mutations(&interpreter)
        .into_iter()
        .find_map(|mutation| match mutation {
            MutationOperation::WriteMemoryBytes { address, old, new } => Some((address, old, new)),
            _ => None,
        })
        .expect("the bytes the answer wrote");
    assert_eq!(bytes.0, 0x2000);
    assert_eq!(bytes.1, vec![0xff, 0xff, 0xff], "the bytes it replaced");
    assert_eq!(bytes.2, vec![b'h', b'i', 0], "and the bytes it left");
    assert_eq!(
        interpreter.get_memory().read_bytes(0x2000, 3).unwrap(),
        &[b'h', b'i', 0]
    );
}

#[test]
fn a_pushed_return_address_reports_the_address_it_wrote() {
    let mut interpreter = with_history(
        "    org $1000
    bsr sub
    nop
sub:
    rts
",
    );
    step(&mut interpreter, "the bsr");
    let after_bsr = interpreter.get_last_steps(1)[0].get_pc();
    match last_mutations(&interpreter).as_slice() {
        [MutationOperation::WriteMemory {
            address,
            old,
            new,
            size,
        }, MutationOperation::PushCall { .. }, MutationOperation::WriteRegister {
            register: stack_pointer,
            ..
        }] => {
            assert_eq!(*size, Size::Long);
            assert_eq!(
                *stack_pointer,
                RegisterOperand::Address(7),
                "the stack pointer moved down too"
            );
            assert_eq!(*old, 0xffffffff, "the untouched stack below the pointer");
            assert_eq!(
                *new,
                interpreter.get_memory().read_long(*address).unwrap(),
                "the return address, which is what is there now"
            );
            assert_eq!(
                *new as usize,
                after_bsr + 4,
                "the instruction after the four-byte bsr"
            );
        }
        other => panic!(
            "expected a memory write, a pushed call and the stack pointer, got {:?}",
            other
        ),
    }
}

#[test]
fn the_program_counter_is_restored_from_the_step_and_is_no_write() {
    // this Core records no program-counter write: the step carries the address
    // the instruction ran at and undo puts that back, so there is no write
    // entry for it to report a new value on.
    let mut interpreter = with_history(
        "    org $1000
    bra there
    nop
there:
    nop
",
    );
    step(&mut interpreter, "the branch");
    let step_pc = interpreter.get_last_steps(1)[0].get_pc();
    assert_eq!(step_pc, 0x1000, "the address the branch ran at");
    assert!(
        last_mutations(&interpreter).is_empty(),
        "a branch writes no register and no memory, the program counter included"
    );
    let jumped_to = interpreter.get_pc();
    assert_ne!(jumped_to, step_pc, "the branch moved the program counter");
    let undone = interpreter.undo().expect("the branch to be undone");
    assert_eq!(undone.get_pc(), step_pc);
    assert_eq!(
        interpreter.get_pc(),
        step_pc,
        "undo puts back the address the step carries"
    );
}

// ---------------------------------------------------------------------------
// 2. The values cross to JavaScript without loss
// ---------------------------------------------------------------------------

#[test]
fn every_value_crosses_as_an_unsigned_number() {
    // a 32 bit Core reports ints the way its getters do: a register of all ones
    // is 4294967295 and never -1, and never a string.
    let mut interpreter = with_history(
        "    org $1000
    move.l #$ffffffff,d0
    move.l d0,$2000
",
    );
    step(&mut interpreter, "the register move");
    let register = as_json(interpreter.get_last_steps(1)[0]);
    let write = &register["mutations"][0]["value"];
    assert_eq!(write["old"].as_u64(), Some(0));
    assert_eq!(
        write["new"].as_u64(),
        Some(0xffffffff),
        "the whole register, unsigned"
    );
    assert!(write["new"].is_u64(), "a number, not a string");

    step(&mut interpreter, "the memory move");
    let memory = as_json(interpreter.get_last_steps(1)[0]);
    let write = &memory["mutations"][0]["value"];
    assert_eq!(write["old"].as_u64(), Some(0xffffffff));
    assert_eq!(write["new"].as_u64(), Some(0xffffffff));
    assert!(write["new"].is_u64());
}

#[test]
fn a_run_of_bytes_crosses_as_an_array_of_numbers() {
    let mut interpreter = with_history(
        "    org $1000
    nop
",
    );
    step(&mut interpreter, "the nop");
    interpreter
        .set_memory_bytes(0x2000, &[0xde, 0xad])
        .expect("the bytes");
    let write = &as_json(interpreter.get_last_steps(1)[0])["mutations"][0]["value"];
    assert_eq!(write["old"], serde_json::json!([255, 255]));
    assert_eq!(write["new"], serde_json::json!([0xde, 0xad]));
}

// ---------------------------------------------------------------------------
// 3. The shape is additive
// ---------------------------------------------------------------------------

/// A program that writes a register, a sized memory cell and a call stack, so
/// that the whole history holds every kind of mutation there is.
const EVERY_KIND: &str = "    org $1000
    movea.l #$3000,a0
    move.l #$11223344,d0
    move.w d0,(a0)
    bsr sub
    nop
sub:
    move.b #$7f,d1
    rts
";

#[test]
fn every_write_of_every_step_carries_both_values_and_its_old_fields() {
    let mut interpreter = with_history(EVERY_KIND);
    for _ in 0..7 {
        if interpreter.step().is_err() {
            break;
        }
    }
    let mut register_writes = 0;
    let mut memory_writes = 0;
    for step in interpreter.get_last_steps(100) {
        for mutation in write_mutations_json(step) {
            let kind = mutation["type"].as_str().expect("a type");
            let value = &mutation["value"];
            assert!(
                !value["old"].is_null(),
                "{} keeps its old value: {}",
                kind,
                mutation
            );
            assert!(
                !value["new"].is_null(),
                "{} reports the value it wrote: {}",
                kind,
                mutation
            );
            match kind {
                "WriteRegister" => {
                    register_writes += 1;
                    assert!(!value["register"].is_null(), "and keeps its register");
                    assert!(!value["size"].is_null(), "and its size");
                }
                "WriteMemory" => {
                    memory_writes += 1;
                    assert!(!value["address"].is_null(), "and keeps its address");
                    assert!(!value["size"].is_null(), "and its size");
                }
                "WriteMemoryBytes" => {
                    assert!(!value["address"].is_null(), "and keeps its address");
                }
                other => panic!("unexpected write {}", other),
            }
        }
    }
    assert!(register_writes >= 3, "the program wrote registers");
    assert!(memory_writes >= 2, "and memory");
}

#[test]
fn a_pokes_writes_are_unchanged_and_its_mutations_carry_the_new_value_too() {
    let mut interpreter = with_history(
        "    org $1000
    move.l #1,d0
    move.l #2,d0
",
    );
    step(&mut interpreter, "the first move");
    interpreter.begin_poke().expect("a poke");
    interpreter.set_register_value(RegisterOperand::Data(0), 0x99, Size::Byte);
    interpreter
        .write_memory_bytes(0x2000, &[9, 9])
        .expect("the bytes");
    assert!(interpreter.end_poke().expect("the poke ends"));
    let json = as_json(interpreter.get_last_steps(1)[0]);

    let writes = json["writes"].as_array().expect("a writes list");
    assert_eq!(writes.len(), 2, "the poke's own writes are unchanged");
    assert_eq!(writes[0]["type"], "register");
    assert_eq!(writes[0]["name"], "d0");
    assert_eq!(writes[0]["old"], 1);
    assert_eq!(writes[0]["new"], 0x99);
    assert_eq!(writes[1]["old"], serde_json::json!([255, 255]));
    assert_eq!(writes[1]["new"], serde_json::json!([9, 9]));

    let mutations = json["mutations"].as_array().expect("a mutations list");
    assert_eq!(mutations[0]["value"]["old"], 1);
    assert_eq!(
        mutations[0]["value"]["new"], 0x99,
        "a poke journals the same three shapes, so its mutations carry new as well"
    );
    assert_eq!(mutations[1]["value"]["old"], serde_json::json!([255, 255]));
    assert_eq!(mutations[1]["value"]["new"], serde_json::json!([9, 9]));
}

// ---------------------------------------------------------------------------
// 4. The value is captured at the store
// ---------------------------------------------------------------------------

#[test]
fn a_register_written_twice_reports_each_store_and_not_the_value_it_ends_on() {
    // `move.l (a0)+,(a0)+` increments a0 twice, once for the read and once for
    // the write. Reading the register back at the end of the step would report
    // the same value for both; each write reports the store it was.
    let mut interpreter = with_history(
        "    org $1000
    movea.l #$2000,a0
    move.l (a0)+,(a0)+
",
    );
    step(&mut interpreter, "the movea");
    step(&mut interpreter, "the move");
    let writes = last_mutations(&interpreter)
        .into_iter()
        .filter_map(|mutation| match mutation {
            MutationOperation::WriteRegister { old, new, .. } => Some((old, new)),
            _ => None,
        })
        .collect::<Vec<(u32, u32)>>();
    assert_eq!(
        writes,
        vec![(0x2000, 0x2004), (0x2004, 0x2008)],
        "one entry per store, each with what it found and what it left"
    );
    assert_eq!(
        interpreter.get_register_value(RegisterOperand::Address(0), Size::Long),
        0x2008,
        "which is where the register ends"
    );
}

#[test]
fn a_write_keeps_what_it_wrote_when_a_later_instruction_overwrites_it() {
    let mut interpreter = with_history(
        "    org $1000
    move.w #$1111,$2000
    move.w #$2222,$2000
",
    );
    step(&mut interpreter, "the first move");
    step(&mut interpreter, "the second move");
    let history = interpreter.get_last_steps(2);
    let newest = last_write(history[0]);
    let older = last_write(history[1]);
    assert_eq!(
        older,
        (0xffff, 0x1111),
        "what the first write found and left"
    );
    assert_eq!(
        newest,
        (0x1111, 0x2222),
        "and the second, read at its own store"
    );
}

/// The old and new value of the newest memory write of a step.
fn last_write(step: &ExecutionStep) -> (u32, u32) {
    step.get_mutations()
        .iter()
        .find_map(|mutation| match mutation {
            MutationOperation::WriteMemory { old, new, .. } => Some((*old, *new)),
            _ => None,
        })
        .expect("a memory write")
}

#[test]
fn an_interpreter_that_keeps_no_history_records_nothing() {
    // the new value is read inside the same gate as the old one, so a run that
    // keeps no history pays for neither.
    let mut interpreter = without_history(
        "    org $1000
    move.l #$aabbccdd,d0
    move.w d0,$2000
",
    );
    step(&mut interpreter, "the register move");
    step(&mut interpreter, "the memory move");
    assert!(
        interpreter
            .get_last_steps(10)
            .iter()
            .all(|step| step.get_mutations().is_empty()),
        "nothing is recorded when no history is kept"
    );
}
