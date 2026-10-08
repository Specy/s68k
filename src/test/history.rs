//! The history once it is full: a step that is written over the oldest one
//! takes that one's slot, so these check that the steps the history answers
//! are the ones it should, with the ids, locations and undo of a history that
//! never wrapped.

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
