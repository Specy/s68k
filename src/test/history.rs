//! The history once it is full: a step that is written over the oldest one
//! takes that one's slot, so these check that the steps the history answers
//! are the ones it should, with the ids, locations and undo of a history that
//! never wrapped, and the start of a history that never filled.

use serde_json::Value;

use crate::debugger::{ExecutionStep, ExecutionStepKind};
use crate::instructions::{RegisterOperand, Size};
use crate::interpreter::{Interpreter, InterpreterOptions};

const LOOP: &str = "
    ORG $1000
START:
    MOVEQ #0,D0
LOOP:
    ADDQ.L #1,D0
    MOVE.L D0,D1
    BRA LOOP
    END START
";

/// An Interpreter over `LOOP` that keeps `history_size` steps.
fn with_history(history_size: usize) -> Interpreter {
    let assembly = crate::assembler::assemble_source(LOOP);
    let program = assembly
        .program
        .unwrap_or_else(|| panic!("LOOP did not assemble"));
    Interpreter::new(
        program,
        Some(InterpreterOptions {
            keep_history: true,
            history_size,
        }),
    )
}

/// The ids of the steps the history answers, newest first.
fn ids(interpreter: &Interpreter) -> Vec<u64> {
    interpreter
        .get_last_steps(usize::MAX)
        .iter()
        .map(|step| step.get_id())
        .collect()
}

fn as_json(step: &ExecutionStep) -> Value {
    serde_json::to_value(step).unwrap()
}

#[test]
fn a_full_history_keeps_its_newest_steps() {
    let mut interpreter = with_history(5);
    for _ in 0..12 {
        interpreter.step().unwrap();
    }
    assert_eq!(ids(&interpreter), vec![12, 11, 10, 9, 8]);
    assert_eq!(interpreter.get_last_step_id(), 12);
}

#[test]
fn undo_walks_back_through_a_history_that_has_wrapped() {
    let mut interpreter = with_history(5);
    for _ in 0..12 {
        interpreter.step().unwrap();
    }
    let mut undone = vec![];
    while interpreter.can_undo() {
        undone.push(interpreter.undo().unwrap().get_id());
    }
    assert_eq!(undone, vec![12, 11, 10, 9, 8]);
    assert!(!interpreter.can_undo());

    //the slots the undone steps held are reused by the steps that follow
    interpreter.step().unwrap();
    interpreter.step().unwrap();
    assert_eq!(ids(&interpreter), vec![14, 13]);
}

#[test]
fn a_step_kept_after_a_wrap_is_the_same_step_as_in_an_unbounded_history() {
    let mut unbounded = with_history(1000);
    let mut bounded = with_history(3);
    for _ in 0..40 {
        unbounded.step().unwrap();
        bounded.step().unwrap();
    }
    let all: Vec<(u64, Value)> = unbounded
        .get_last_steps(usize::MAX)
        .iter()
        .map(|step| (step.get_id(), as_json(step)))
        .collect();
    let kept = bounded.get_last_steps(usize::MAX);
    assert_eq!(kept.len(), 3);
    for step in kept {
        let (_, expected) = all
            .iter()
            .find(|(id, _)| *id == step.get_id())
            .unwrap_or_else(|| panic!("step {} is not in the unbounded history", step.get_id()));
        assert_eq!(&as_json(step), expected);
    }
}

#[test]
fn a_poke_after_a_wrap_is_the_newest_step_and_undo_takes_it_back() {
    let mut interpreter = with_history(3);
    for _ in 0..5 {
        interpreter.step().unwrap();
    }
    interpreter.begin_poke().unwrap();
    interpreter.set_register_value(RegisterOperand::Data(3), 0x55, Size::Long);
    assert!(interpreter.end_poke().unwrap());

    assert_eq!(ids(&interpreter), vec![6, 5, 4]);
    assert_eq!(
        interpreter.get_last_steps(1)[0].get_kind(),
        ExecutionStepKind::Poke
    );
    let undone = interpreter.undo().unwrap();
    assert_eq!(undone.get_kind(), ExecutionStepKind::Poke);
    assert_eq!(interpreter.get_last_step_id(), 5);
}

#[test]
fn a_program_that_has_not_run_has_nothing_to_undo() {
    let interpreter = with_history(5);
    assert!(!interpreter.can_undo());
    assert!(interpreter.get_last_steps(usize::MAX).is_empty());
    assert_eq!(interpreter.get_last_step_id(), 0);
}

#[test]
fn undo_stops_at_the_first_step_of_a_history_that_never_filled() {
    let mut interpreter = with_history(100);
    let entry = interpreter.get_pc();
    for _ in 0..3 {
        interpreter.step().unwrap();
    }
    let mut undone = vec![];
    while interpreter.can_undo() {
        undone.push(interpreter.undo().unwrap().get_id());
    }
    assert_eq!(undone, vec![3, 2, 1]);
    assert_eq!(interpreter.get_pc(), entry);
    assert!(interpreter.undo().is_err());

    //and the program runs again from where it started
    interpreter.step().unwrap();
    assert_eq!(ids(&interpreter), vec![4]);
}

#[test]
fn an_interpreter_without_history_never_has_anything_to_undo() {
    let assembly = crate::assembler::assemble_source(LOOP);
    let mut interpreter = Interpreter::new(
        assembly.program.unwrap(),
        Some(InterpreterOptions {
            keep_history: false,
            history_size: 0,
        }),
    );
    interpreter.step().unwrap();
    assert!(!interpreter.can_undo());
}

/// Each step keeps the words of the instruction it ran, so it says what ran
/// after a later write has changed them in memory, which is what the History
/// labels it with (batch 1 review, the History's "Ran …"). A Poke ran
/// nothing.
#[test]
fn a_step_says_what_it_ran_after_its_code_has_changed() {
    let assembly = crate::assembler::assemble_source(
        "    org $1000
start:
    add.l #5,d1
    move.w #$4e71,start
    bra start
",
    );
    let program = assembly.program.expect("the program assembles");
    let mut interpreter = Interpreter::new(
        program,
        Some(InterpreterOptions {
            keep_history: true,
            history_size: 10,
        }),
    );
    for _ in 0..4 {
        interpreter.step().unwrap();
    }
    interpreter.begin_poke().unwrap();
    interpreter.set_register_value(RegisterOperand::Data(2), 7, Size::Long);
    interpreter.end_poke().unwrap();
    let steps = interpreter.get_last_steps(5);
    assert_eq!(steps.len(), 5);
    assert_eq!(steps[0].get_kind(), ExecutionStepKind::Poke);
    assert!(interpreter.ran_by(steps[0]).is_none(), "a Poke ran nothing");
    assert_eq!(steps[0].get_ran(), crate::debugger::Ran::Nothing);
    // The nop the store wrote ran from memory, decoded as the program ran.
    assert!(matches!(
        steps[1].get_ran(),
        crate::debugger::Ran::Decoded { len: 2, .. }
    ));
    assert_eq!(interpreter.ran_by(steps[1]).unwrap().text, "NOP");
    let store = interpreter.ran_by(steps[3]).expect("the store ran");
    assert_eq!(store.text, "MOVE.W #$4E71,start.W");
    assert_eq!(store.address, 0x1002);
    assert!(matches!(steps[4].get_ran(), crate::debugger::Ran::Assembled(0)));
    let first = interpreter.ran_by(steps[4]).expect("the addq ran");
    assert_eq!(first.text, "ADDQ.L #5,D1");
    assert_eq!(first.bytes, [0x5a, 0x81]);
    assert_eq!((first.address, first.size), (0x1000, 2));
    // Memory says otherwise now: the store wrote a nop over it.
    assert_eq!(interpreter.disassemble_at(0x1000).text, "NOP");
    // Undo takes the steps back, words and all.
    for _ in 0..3 {
        interpreter.undo().unwrap();
    }
    let undone = interpreter.undo().unwrap();
    assert_eq!(interpreter.ran_by(&undone).unwrap().text, "MOVE.W #$4E71,start.W");
}
