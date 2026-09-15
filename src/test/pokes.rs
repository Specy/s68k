//! Pokes: a register or memory value the host changes between two
//! instructions, recorded in the Interpreter's own history as one step of its
//! own and undone by it.
//!
//! One test per rule of the Poke contract (the asm-editor's
//! `docs/design/pokes.md` and ADR 0022): the transaction, what the existing
//! setters do inside it and outside it, what the step carries, where it sits in
//! the history and what undoing it puts back.

use serde_json::Value;

use crate::assembler::instructions::encoded::RegisterOperand;
use crate::assembler::program::Program;
use crate::debugger::{ExecutionStep, ExecutionStepKind};
use crate::instructions::Size;
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

/// An Interpreter over `code` that keeps a history, which is what undo and
/// therefore a Poke need.
fn with_history(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: true,
            history_size: 100,
        }),
    )
}

/// An Interpreter over `code` whose history holds `size` steps.
fn with_history_of(code: &str, size: usize) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: true,
            history_size: size,
        }),
    )
}

/// Two instructions that write nothing the tests care about, so that a Poke has
/// instructions around it.
const TWO_MOVES: &str = "    org $1000
    move.l #1,d0
    move.l #2,d0
";

fn data(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
}

fn set_data(interpreter: &mut Interpreter, register: u8, value: u32) {
    interpreter.set_register_value(RegisterOperand::Data(register), value, Size::Long);
}

fn as_json(step: &ExecutionStep) -> Value {
    serde_json::to_value(step).expect("a step serialises")
}

// ---------------------------------------------------------------------------
// 1. The transaction
// ---------------------------------------------------------------------------

#[test]
fn a_poke_is_begun_and_ended() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter.step().expect("the first move");
    assert!(!interpreter.is_poking());
    interpreter.begin_poke().expect("a poke can be begun");
    assert!(interpreter.is_poking());
    set_data(&mut interpreter, 3, 0x1234);
    assert!(interpreter.end_poke().expect("the poke can be ended"));
    assert!(!interpreter.is_poking());
}

#[test]
fn beginning_a_poke_inside_a_poke_is_refused() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter.begin_poke().expect("the first poke");
    assert!(
        interpreter.begin_poke().is_err(),
        "a poke inside a poke is refused"
    );
    assert!(
        interpreter.end_poke().is_ok(),
        "the refusal left the first poke open"
    );
}

#[test]
fn ending_a_poke_that_was_never_begun_is_refused() {
    let mut interpreter = with_history(TWO_MOVES);
    assert!(interpreter.end_poke().is_err(), "no poke is open");
    interpreter.begin_poke().expect("a poke");
    interpreter.end_poke().expect("ended once");
    assert!(
        interpreter.end_poke().is_err(),
        "and it cannot be ended twice"
    );
}

#[test]
fn beginning_a_poke_while_an_instruction_is_executing_is_refused() {
    // an instruction that raised an Interrupt has not finished: it is waiting
    // for the host to answer it, and what that answer writes is its own.
    let mut interpreter = with_history(
        "    org $1000
    move.b #4,d0
    trap #15
",
    );
    interpreter.step().expect("the move");
    interpreter.step().expect("the trap");
    assert_eq!(*interpreter.get_status(), InterpreterStatus::Interrupt);
    assert!(
        interpreter.begin_poke().is_err(),
        "a poke while an instruction is executing is refused"
    );
    interpreter
        .answer_interrupt(crate::instructions::InterruptResult::ReadNumber(7))
        .expect("the answer");
    interpreter
        .begin_poke()
        .expect("and is allowed once the instruction has finished");
    interpreter.end_poke().expect("the poke ends");
}

// ---------------------------------------------------------------------------
// 2. The setters journal inside a transaction and stay direct outside one
// ---------------------------------------------------------------------------

#[test]
fn a_host_register_write_outside_a_poke_records_nothing() {
    // 2.2.1 appended this write to the last instruction's mutations, so undoing
    // that instruction reverted the host's write too.
    let mut interpreter = with_history(TWO_MOVES);
    interpreter.step().expect("the first move");
    set_data(&mut interpreter, 5, 0xbeef);
    let step = interpreter.undo().expect("the first move to be undone");
    assert_eq!(
        step.get_mutations().len(),
        1,
        "the step holds what the instruction wrote and nothing else"
    );
    assert_eq!(
        data(&interpreter, 5),
        0xbeef,
        "the host's write outlives the instruction it was made after"
    );
}

#[test]
fn a_host_memory_write_outside_a_poke_records_nothing() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter.step().expect("the first move");
    interpreter
        .write_memory_bytes(0x2000, &[1, 2, 3, 4])
        .expect("the write");
    interpreter.undo().expect("the first move to be undone");
    assert_eq!(
        interpreter.get_memory().read_bytes(0x2000, 4).unwrap(),
        &[1, 2, 3, 4],
        "the bytes survive the undo of the instruction before them"
    );
    assert!(
        !interpreter.can_undo() || interpreter.get_last_steps(1)[0].get_writes().is_empty(),
        "and nothing was recorded for them"
    );
}

#[test]
fn what_an_interrupt_answer_writes_still_belongs_to_the_instruction() {
    // the answer to an Interrupt is the host writing, but the instruction that
    // raised it has not finished: undoing it takes the answer back with it.
    let mut interpreter = with_history(
        "    org $1000
    move.b #4,d0
    trap #15
",
    );
    interpreter.step().expect("the move");
    interpreter.step().expect("the trap");
    interpreter
        .answer_interrupt(crate::instructions::InterruptResult::ReadNumber(42))
        .expect("the answer");
    assert_eq!(data(&interpreter, 1), 42);
    interpreter.undo().expect("the trap to be undone");
    assert_eq!(
        data(&interpreter, 1),
        0,
        "undoing the trap takes back what answering it wrote"
    );
}

#[test]
fn a_poke_journals_both_setters() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter.begin_poke().expect("a poke");
    set_data(&mut interpreter, 2, 0x11);
    interpreter
        .write_memory_bytes(0x3000, &[0xaa, 0xbb])
        .expect("the write");
    assert!(interpreter.end_poke().expect("the poke ends"));
    let step = interpreter.undo().expect("the poke to be undone");
    assert_eq!(step.get_writes().len(), 2, "one write per value written");
    assert_eq!(data(&interpreter, 2), 0, "the register is back");
    assert_eq!(
        interpreter.get_memory().read_bytes(0x3000, 2).unwrap(),
        //memory starts filled with $FF, which is what the poke overwrote
        &[0xff, 0xff],
        "and so are the bytes"
    );
}

// ---------------------------------------------------------------------------
// 3. A write that changes nothing, and a poke that wrote nothing
// ---------------------------------------------------------------------------

#[test]
fn a_poke_that_wrote_nothing_records_no_step() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter.step().expect("the first move");
    let before = interpreter.get_last_step_id();
    interpreter.begin_poke().expect("a poke");
    assert!(
        !interpreter.end_poke().expect("the poke ends"),
        "an empty poke records nothing"
    );
    assert_eq!(interpreter.get_last_step_id(), before, "and takes no id");
}

#[test]
fn a_write_that_leaves_the_value_where_it_was_records_nothing() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter
        .step()
        .expect("the first move, which writes 1 to d0");
    assert_eq!(data(&interpreter, 0), 1);
    interpreter.begin_poke().expect("a poke");
    set_data(&mut interpreter, 0, 1);
    interpreter
        .write_memory_bytes(0x4000, &[0xff, 0xff, 0xff])
        .expect("bytes that are already there");
    assert!(
        !interpreter.end_poke().expect("the poke ends"),
        "writing what is already there is no write at all"
    );
    assert_eq!(
        interpreter.get_last_steps(1)[0].get_kind(),
        ExecutionStepKind::Instruction,
        "the newest step is still the instruction"
    );
}

#[test]
fn a_poke_records_one_step_however_many_writes_it_holds() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter.step().expect("the first move");
    let before = interpreter.get_last_steps(100).len();
    interpreter.begin_poke().expect("a poke");
    set_data(&mut interpreter, 1, 1);
    set_data(&mut interpreter, 2, 2);
    interpreter
        .write_memory_bytes(0x5000, &[9])
        .expect("a byte");
    assert!(interpreter.end_poke().expect("the poke ends"));
    assert_eq!(
        interpreter.get_last_steps(100).len(),
        before + 1,
        "three writes, one step"
    );
}

// ---------------------------------------------------------------------------
// 4. What the step says it is, and what it carries
// ---------------------------------------------------------------------------

#[test]
fn an_instruction_step_says_it_is_an_instruction() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter.step().expect("the first move");
    let step = interpreter.undo().expect("a step");
    assert_eq!(step.get_kind(), ExecutionStepKind::Instruction);
    let json = as_json(&step);
    assert_eq!(json["kind"], "instruction", "the kind is always written");
    assert_eq!(
        json["writes"].as_array().expect("a writes list").len(),
        0,
        "an instruction wrote no poke values"
    );
}

#[test]
fn a_poke_step_carries_the_old_and_the_new_value_of_every_register_it_wrote() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter
        .step()
        .expect("the first move, which writes 1 to d0");
    interpreter.begin_poke().expect("a poke");
    set_data(&mut interpreter, 0, 0x99);
    interpreter.set_register_value(RegisterOperand::Address(7), 0x1000, Size::Long);
    assert!(interpreter.end_poke().expect("the poke ends"));
    let json = as_json(interpreter.get_last_steps(1)[0]);
    assert_eq!(json["kind"], "poke");
    let writes = json["writes"].as_array().expect("a writes list");
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0]["type"], "register");
    assert_eq!(writes[0]["name"], "d0", "named as the editor spells it");
    assert_eq!(writes[0]["old"], 1, "what the register held before");
    assert_eq!(writes[0]["new"], 0x99, "and what it holds now");
    assert_eq!(writes[1]["name"], "a7");
    assert_eq!(writes[1]["new"], 0x1000);
}

#[test]
fn a_poke_step_carries_the_old_and_the_new_bytes_of_the_memory_it_wrote() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter
        .write_memory_bytes(0x6000, &[1, 2, 3])
        .expect("what is there before the poke, written outside any poke");
    interpreter.begin_poke().expect("a poke");
    interpreter
        .write_memory_bytes(0x6000, &[4, 5, 6])
        .expect("the poke's write");
    assert!(interpreter.end_poke().expect("the poke ends"));
    let json = as_json(interpreter.get_last_steps(1)[0]);
    let writes = json["writes"].as_array().expect("a writes list");
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0]["type"], "memory");
    assert_eq!(writes[0]["address"], 0x6000);
    assert_eq!(writes[0]["old"], serde_json::json!([1, 2, 3]));
    assert_eq!(writes[0]["new"], serde_json::json!([4, 5, 6]));
}

#[test]
fn a_register_written_twice_in_one_poke_reports_the_value_it_ends_on() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter
        .step()
        .expect("the first move, which writes 1 to d0");
    interpreter.begin_poke().expect("a poke");
    set_data(&mut interpreter, 0, 7);
    set_data(&mut interpreter, 0, 8);
    assert!(interpreter.end_poke().expect("the poke ends"));
    let json = as_json(interpreter.get_last_steps(1)[0]);
    let writes = json["writes"].as_array().expect("a writes list");
    assert_eq!(writes.len(), 1, "one register, one write");
    assert_eq!(writes[0]["old"], 1);
    assert_eq!(writes[0]["new"], 8);
    interpreter.undo().expect("the poke to be undone");
    assert_eq!(data(&interpreter, 0), 1, "undo puts back what it found");
}

// ---------------------------------------------------------------------------
// 5. Where the step sits, and the slot it takes
// ---------------------------------------------------------------------------

#[test]
fn a_poke_sits_among_the_instructions_in_the_history() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter.step().expect("the first move");
    interpreter.begin_poke().expect("a poke");
    set_data(&mut interpreter, 4, 4);
    assert!(interpreter.end_poke().expect("the poke ends"));
    interpreter.step().expect("the second move");
    let steps = interpreter.get_last_steps(3);
    let kinds: Vec<ExecutionStepKind> = steps.iter().map(|step| step.get_kind()).collect();
    assert_eq!(
        kinds,
        vec![
            ExecutionStepKind::Instruction,
            ExecutionStepKind::Poke,
            ExecutionStepKind::Instruction
        ],
        "newest first, as the history has always been read"
    );
    assert!(interpreter.can_undo());
}

#[test]
fn a_poke_takes_one_slot_of_the_history() {
    let mut interpreter = with_history_of(TWO_MOVES, 2);
    interpreter.step().expect("the first move");
    interpreter.step().expect("the second move");
    interpreter.begin_poke().expect("a poke");
    set_data(&mut interpreter, 6, 6);
    assert!(interpreter.end_poke().expect("the poke ends"));
    let steps = interpreter.get_last_steps(10);
    assert_eq!(steps.len(), 2, "the history still holds two steps");
    assert_eq!(steps[0].get_kind(), ExecutionStepKind::Poke);
    assert_eq!(
        steps[1].get_kind(),
        ExecutionStepKind::Instruction,
        "the poke pushed the oldest instruction out"
    );
}

#[test]
fn an_interpreter_that_keeps_no_history_applies_a_poke_and_records_nothing() {
    let mut interpreter = Interpreter::new(
        assemble(TWO_MOVES),
        Some(InterpreterOptions {
            keep_history: false,
            history_size: 0,
        }),
    );
    interpreter.begin_poke().expect("a poke");
    set_data(&mut interpreter, 3, 0x42);
    assert!(
        !interpreter.end_poke().expect("the poke ends"),
        "there is no history to record it in"
    );
    assert_eq!(data(&interpreter, 3), 0x42, "but the value is written");
}

// ---------------------------------------------------------------------------
// 6. Undo
// ---------------------------------------------------------------------------

#[test]
fn undoing_a_poke_puts_back_every_value_it_wrote_and_touches_nothing_else() {
    let mut interpreter = with_history(
        "    org $1000
    bsr routine
    move.l #5,d0
routine:
    move.l #3,d1
    cmp.l #3,d1
    rts
",
    );
    interpreter.step().expect("the bsr");
    interpreter.step().expect("the move inside the routine");
    interpreter.step().expect("the cmp, which sets the flags");
    let pc = interpreter.get_pc();
    let sr = interpreter.get_sr();
    let flags = interpreter.get_flags_as_array();
    let call_stack = interpreter.get_pretty_call_stack().len();
    let status = *interpreter.get_status();
    let d1 = data(&interpreter, 1);

    interpreter.begin_poke().expect("a poke");
    set_data(&mut interpreter, 1, 0xdead);
    interpreter
        .write_memory_bytes(0x7000, &[7, 7])
        .expect("two bytes");
    assert!(interpreter.end_poke().expect("the poke ends"));
    assert!(interpreter.can_undo(), "a poke can be undone like anything");

    let step = interpreter.undo().expect("the poke to be undone");
    assert_eq!(step.get_kind(), ExecutionStepKind::Poke, "undo answers it");
    assert_eq!(data(&interpreter, 1), d1, "the register is back");
    assert_eq!(
        interpreter.get_memory().read_bytes(0x7000, 2).unwrap(),
        &[0xff, 0xff],
        "and the bytes are back"
    );
    assert_eq!(interpreter.get_pc(), pc, "the program counter did not move");
    assert_eq!(interpreter.get_sr(), sr, "the status register is untouched");
    assert_eq!(interpreter.get_flags_as_array(), flags, "so are the flags");
    assert_eq!(
        interpreter.get_pretty_call_stack().len(),
        call_stack,
        "and the call stack"
    );
    assert_eq!(*interpreter.get_status(), status);
}

// ---------------------------------------------------------------------------
// 7. A poke's identity is its own
// ---------------------------------------------------------------------------

#[test]
fn a_poke_has_a_step_id_of_its_own() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter.step().expect("the first move");
    let instruction_id = interpreter.get_last_step_id();
    let instruction_pc = interpreter.get_last_steps(1)[0].get_pc();
    interpreter.begin_poke().expect("a poke");
    set_data(&mut interpreter, 7, 7);
    assert!(interpreter.end_poke().expect("the poke ends"));
    let poke = interpreter.get_last_steps(1)[0];
    assert!(
        poke.get_id() > instruction_id,
        "the poke took an id of its own, after the instruction's"
    );
    assert_eq!(
        interpreter.get_last_step_id(),
        poke.get_id(),
        "and the newest id is the poke's"
    );
    assert_ne!(
        poke.get_pc(),
        instruction_pc,
        "it is not the instruction's step in disguise"
    );
    assert_eq!(
        poke.get_pc(),
        interpreter.get_pc(),
        "its pc is where the program counter is, because no instruction ran"
    );
    assert!(
        poke.get_location().is_none(),
        "and it was written on no source line"
    );
}

// ---------------------------------------------------------------------------
// 8. Several steps in a row
// ---------------------------------------------------------------------------

#[test]
fn two_pokes_in_a_row_are_two_steps() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter.begin_poke().expect("the first poke");
    set_data(&mut interpreter, 1, 1);
    assert!(interpreter.end_poke().expect("it ends"));
    interpreter.begin_poke().expect("the second poke");
    set_data(&mut interpreter, 2, 2);
    assert!(interpreter.end_poke().expect("it ends"));
    let steps = interpreter.get_last_steps(2);
    assert_eq!(steps.len(), 2);
    assert!(steps
        .iter()
        .all(|step| step.get_kind() == ExecutionStepKind::Poke));
    assert_ne!(steps[0].get_id(), steps[1].get_id());
    interpreter.undo().expect("the second poke");
    assert_eq!(data(&interpreter, 2), 0);
    assert_eq!(data(&interpreter, 1), 1, "the first poke still stands");
    interpreter.undo().expect("the first poke");
    assert_eq!(data(&interpreter, 1), 0);
}

#[test]
fn a_poke_then_an_instruction_is_undone_newest_first() {
    let mut interpreter = with_history(TWO_MOVES);
    interpreter
        .step()
        .expect("the first move, which writes 1 to d0");
    let pc = interpreter.get_pc();
    let d0 = data(&interpreter, 0);

    interpreter.begin_poke().expect("a poke");
    set_data(&mut interpreter, 0, 0x77);
    interpreter
        .write_memory_bytes(0x8000, &[1])
        .expect("one byte");
    assert!(interpreter.end_poke().expect("the poke ends"));
    interpreter
        .step()
        .expect("the second move, which writes 2 to d0");
    assert_eq!(data(&interpreter, 0), 2);

    let undone = interpreter.undo().expect("the instruction");
    assert_eq!(
        undone.get_kind(),
        ExecutionStepKind::Instruction,
        "the instruction is reverted first"
    );
    assert_eq!(data(&interpreter, 0), 0x77, "back to what the poke left");
    let undone = interpreter.undo().expect("the poke");
    assert_eq!(undone.get_kind(), ExecutionStepKind::Poke);
    assert_eq!(data(&interpreter, 0), d0, "and back to before the poke");
    assert_eq!(
        interpreter.get_memory().read_bytes(0x8000, 1).unwrap(),
        &[0xff]
    );
    assert_eq!(
        interpreter.get_pc(),
        pc,
        "exactly the state before the poke"
    );
}
