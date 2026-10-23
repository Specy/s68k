//! The watched range: one range of addresses whose stores the host sees as
//! they happen, each instruction that makes one ending the run or step that
//! ran it with `WatchedWrite`.
//!
//! The range is a memory-mapped device's registers, so every test watches the
//! block a host would, `$E00100` up to `$E00300`, and checks one rule: which
//! stores are recorded and how, where the run stops, how the stop ranks with
//! the other ones, and what a Poke and undo do to the log.

use crate::assembler::program::Program;
use crate::assembler::source::DEFAULT_ENTRY_PATH;
use crate::debugger::MutationOperation;
use crate::instructions::{InterruptResult, RegisterOperand, Size};
use crate::interpreter::{
    Breakpoint, Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError, Termination,
    WatchedWrite,
};

/// The first address watched.
const START: usize = 0xE00100;
/// One past the last address watched.
const END: usize = 0xE00300;

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

/// An Interpreter over `code` that keeps a history and watches `START..END`.
fn watching(code: &str) -> Interpreter {
    let mut interpreter = Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: true,
            history_size: 100,
        }),
    );
    interpreter
        .watch_memory(START, END)
        .expect("the block is a range of memory");
    interpreter
}

fn write(address: usize, size: usize, value: u32) -> WatchedWrite {
    WatchedWrite {
        address,
        size,
        value,
    }
}

fn data(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
}

fn bytes(interpreter: &Interpreter, address: usize, length: usize) -> Vec<u8> {
    interpreter
        .get_memory()
        .read_bytes(address, length)
        .expect("in memory")
        .to_vec()
}

// ---------------------------------------------------------------------------
// 1. What is recorded
// ---------------------------------------------------------------------------

#[test]
fn a_byte_a_word_and_a_long_are_recorded_at_their_size() {
    let mut interpreter = watching(
        "    org $1000
    move.b #$41,$E00110
    move.w #440,$E00100
    move.l #$12345678,$E00180
",
    );
    for expected in [
        write(0xE00110, 1, 0x41),
        write(0xE00100, 2, 440),
        write(0xE00180, 4, 0x1234_5678),
    ] {
        assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
        assert_eq!(interpreter.take_watched_writes(), vec![expected]);
    }
    assert_eq!(bytes(&interpreter, 0xE00180, 4), vec![0x12, 0x34, 0x56, 0x78]);
}

#[test]
fn each_register_of_a_movem_is_a_write_of_its_own_in_order() {
    let mut interpreter = watching(
        "    org $1000
    move.l #1,d0
    move.l #2,d1
    move.l #3,a0
    lea $E00110,a1
    movem.l d0-d1/a0,(a1)
    movem.w d0-d1,$E00120
",
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![
            write(0xE00110, 4, 1),
            write(0xE00114, 4, 2),
            write(0xE00118, 4, 3),
        ],
        "one write a register, in the order they were stored"
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![write(0xE00120, 2, 1), write(0xE00122, 2, 2)]
    );
}

#[test]
fn a_read_modify_write_instruction_records_its_one_store() {
    let mut interpreter = watching(
        "    org $1000
    move.w #9,$E00100
    addq.w #1,$E00100
    bset #0,$E00110
    clr.b $E00111
",
    );
    interpreter.run().unwrap();
    interpreter.take_watched_writes();
    let mut writes = vec![];
    while interpreter.run().unwrap() == InterpreterStatus::WatchedWrite {
        writes.extend(interpreter.take_watched_writes());
    }
    assert_eq!(
        writes,
        vec![
            write(0xE00100, 2, 10),
            // memory starts filled with $FF, and bit 0 of it is already set
            write(0xE00110, 1, 0xFF),
            write(0xE00111, 1, 0),
        ]
    );
}

#[test]
fn a_push_into_the_range_is_a_store_like_any_other() {
    let mut interpreter = watching(
        "    org $1000
    lea $E00110,a7
    pea $1234
    bsr sub
    move.b #9,d0
    trap #15
sub:
    rts
",
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![write(0xE0010C, 4, 0x1234)]
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![write(0xE00108, 4, 0x100C)],
        "the return address bsr pushes"
    );
}

#[test]
fn a_store_just_outside_either_edge_is_not_recorded() {
    let mut interpreter = watching(
        "    org $1000
    move.b #1,$E000FF
    move.w #1,$E000FE
    move.b #1,$E00300
    move.l #1,$E00300
",
    );
    assert_eq!(
        interpreter.run().unwrap(),
        InterpreterStatus::Terminated,
        "nothing stopped the run"
    );
    assert!(interpreter.take_watched_writes().is_empty());
}

#[test]
fn a_store_that_straddles_an_edge_is_recorded_whole() {
    let mut interpreter = watching(
        "    org $1000
    move.l #$AABBCCDD,$E000FE
    move.l #$11223344,$E002FE
",
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![write(0xE000FE, 4, 0xAABB_CCDD)],
        "it begins before the range, and its last two bytes are in it"
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![write(0xE002FE, 4, 0x1122_3344)]
    );
}

#[test]
fn the_address_recorded_is_the_one_memory_was_written_at() {
    // the 68000 puts 24 bits on its bus, and so does s68k's memory
    let mut interpreter = watching(
        "    org $1000
    move.b #5,$FFE00100
",
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![write(0xE00100, 1, 5)]
    );
}

#[test]
fn the_writes_wait_in_order_until_they_are_taken() {
    // a gate pulsed off and on again is two writes, and neither is lost
    let mut interpreter = watching(
        "    org $1000
    move.b #0,$E00110
    move.b #1,$E00110
    move.w #440,$E00100
",
    );
    for _ in 0..3 {
        assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    }
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![
            write(0xE00110, 1, 0),
            write(0xE00110, 1, 1),
            write(0xE00100, 2, 440),
        ]
    );
    assert!(
        interpreter.take_watched_writes().is_empty(),
        "taking them empties the log"
    );
}

#[test]
fn nothing_is_recorded_while_nothing_is_watched() {
    let mut interpreter = watching(
        "    org $1000
    move.b #1,$E00110
    move.b #2,$E00110
",
    );
    interpreter.unwatch_memory();
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
    assert!(interpreter.take_watched_writes().is_empty());
}

#[test]
fn unwatching_forgets_the_writes_not_taken() {
    let mut interpreter = watching(
        "    org $1000
    move.b #1,$E00110
    move.b #2,$E00110
",
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    interpreter.unwatch_memory();
    assert!(interpreter.take_watched_writes().is_empty());
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
}

#[test]
fn a_range_that_is_empty_or_past_memory_is_refused() {
    let mut interpreter = watching("    org $1000\n    nop\n");
    for (start, end) in [(START, START), (END, START), (0, 0x0100_0001)] {
        assert!(matches!(
            interpreter.watch_memory(start, end),
            Err(RuntimeError::InvalidArgument(_))
        ));
    }
    assert!(interpreter.watch_memory(0xFF_FF00, 0x0100_0000).is_ok());
}

// ---------------------------------------------------------------------------
// 2. Where the run stops
// ---------------------------------------------------------------------------

#[test]
fn watched_write_is_appended_to_the_public_status_values() {
    assert_eq!(InterpreterStatus::Paused as u32, 4);
    assert_eq!(InterpreterStatus::WatchedWrite as u32, 5);
}

#[test]
fn the_run_ends_after_the_storing_instruction_which_counts_as_executed() {
    let mut interpreter = watching(
        "    org $1000
    move.l #7,d0
    move.w d0,$E00100
    move.l #9,d0
",
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    assert_eq!(interpreter.get_pc(), 0x1008, "on the instruction after it");
    assert_eq!(bytes(&interpreter, 0xE00100, 2), vec![0, 7], "the store is done");
    assert_eq!(data(&interpreter, 0), 7, "and nothing after it has run");
    assert!(!interpreter.is_executing());
    assert_eq!(interpreter.get_last_step_id(), 2);
    let step = interpreter.get_last_steps(1)[0].clone();
    assert_eq!(step.get_pc(), 0x1004);
    assert!(matches!(
        step.get_mutations().as_slice(),
        [MutationOperation::WriteMemory {
            address: 0xE00100,
            old: 0xFFFF,
            new: 7,
            ..
        }]
    ));
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
    assert_eq!(data(&interpreter, 0), 9);
}

#[test]
fn a_step_stops_on_the_store_too() {
    let mut interpreter = watching(
        "    org $1000
    move.b #1,$E00110
    nop
",
    );
    assert_eq!(interpreter.step().unwrap(), InterpreterStatus::WatchedWrite);
    assert_eq!(interpreter.step().unwrap(), InterpreterStatus::Terminated);
}

#[test]
fn a_store_by_the_last_instruction_stops_before_the_end() {
    // as a `simhalt` at the bottom does: the next call ends the program
    let mut interpreter = watching(
        "    org $1000
    move.b #1,$E00110
",
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    assert!(interpreter.has_reached_bottom());
    assert!(!interpreter.has_terminated());
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
    assert_eq!(
        interpreter.get_termination(),
        Some(&Termination::EndOfProgram)
    );
}

#[test]
fn a_breakpoint_on_the_next_instruction_is_reported_by_the_next_call() {
    let mut interpreter = watching(
        "    org $1000
    move.b #1,$E00110
    move.l #7,d0
    nop
",
    );
    let breakpoints = [Breakpoint::new(DEFAULT_ENTRY_PATH, 2)];
    assert_eq!(
        interpreter
            .run_with_breakpoints(&breakpoints, None, true)
            .unwrap(),
        InterpreterStatus::WatchedWrite,
        "the watched write first"
    );
    assert_eq!(interpreter.get_pc(), 0x1004);
    interpreter.take_watched_writes();
    assert_eq!(
        interpreter
            .run_with_breakpoints(&breakpoints, None, false)
            .unwrap(),
        InterpreterStatus::Running,
        "then the breakpoint, before its instruction runs"
    );
    assert_eq!(interpreter.get_pc(), 0x1004);
    assert_eq!(data(&interpreter, 0), 0);
    assert_eq!(
        interpreter
            .run_with_breakpoints(&breakpoints, None, true)
            .unwrap(),
        InterpreterStatus::Terminated
    );
    assert_eq!(data(&interpreter, 0), 7);
}

#[test]
fn a_breakpoint_on_the_storing_instruction_stops_before_it() {
    let mut interpreter = watching(
        "    org $1000
    move.b #1,$E00110
    nop
",
    );
    let breakpoints = [Breakpoint::new(DEFAULT_ENTRY_PATH, 1)];
    assert_eq!(
        interpreter
            .run_with_breakpoints(&breakpoints, None, false)
            .unwrap(),
        InterpreterStatus::Running
    );
    assert!(interpreter.take_watched_writes().is_empty());
    assert_eq!(
        interpreter
            .run_with_breakpoints(&breakpoints, None, true)
            .unwrap(),
        InterpreterStatus::WatchedWrite
    );
}

#[test]
fn a_store_on_the_last_instruction_of_a_limit_is_still_reported() {
    let mut interpreter = watching(
        "    org $1000
    nop
    move.b #1,$E00110
    nop
    nop
",
    );
    assert_eq!(
        interpreter.run_with_limit(2).unwrap(),
        InterpreterStatus::WatchedWrite,
        "not the limit: the host must see the write before the next instruction"
    );
    assert!(matches!(
        interpreter.run_with_limit(1),
        Err(RuntimeError::ExecutionLimit(1))
    ));
    assert!(matches!(
        interpreter.run_with_limit(0),
        Err(RuntimeError::ExecutionLimit(0))
    ));
    assert_eq!(
        interpreter.run_with_breakpoints(&[], Some(2), false).unwrap(),
        InterpreterStatus::Terminated
    );
}

#[test]
fn a_failure_after_a_store_ends_the_program_with_the_write_kept() {
    // the second long of the movem lands on the first instruction
    let mut interpreter = Interpreter::new(
        assemble(
            "    org $1000
    lea $FFC,a0
    movem.l d0-d1,(a0)
",
        ),
        Some(InterpreterOptions {
            keep_history: true,
            history_size: 100,
        }),
    );
    interpreter.watch_memory(0xF00, 0x1000).unwrap();
    assert!(matches!(
        interpreter.run(),
        Err(RuntimeError::InstructionAccess { .. })
    ));
    assert_eq!(
        *interpreter.get_status(),
        InterpreterStatus::TerminatedWithException
    );
    assert_eq!(interpreter.take_watched_writes(), vec![write(0xFFC, 4, 0)]);
}

#[test]
fn an_answer_that_writes_into_the_range_stops_too() {
    let mut interpreter = watching(
        "    org $1000
    lea $E00100,a1
    move.b #2,d0
    trap #15
    nop
",
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Interrupt);
    assert!(interpreter.take_watched_writes().is_empty());
    interpreter
        .answer_interrupt(InterruptResult::ReadKeyboardString("hi".to_string()))
        .unwrap();
    assert_eq!(*interpreter.get_status(), InterpreterStatus::WatchedWrite);
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![
            write(0xE00100, 1, b'h' as u32),
            write(0xE00101, 1, b'i' as u32),
            write(0xE00102, 1, 0),
        ],
        "a run of bytes is recorded a byte at a time"
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
}

// ---------------------------------------------------------------------------
// 3. Pokes and undo
// ---------------------------------------------------------------------------

#[test]
fn a_poke_into_the_range_is_recorded_without_a_stop() {
    let mut interpreter = watching(
        "    org $1000
    nop
    nop
",
    );
    interpreter.step().unwrap();
    interpreter.begin_poke().unwrap();
    interpreter
        .write_memory_bytes(0xE002FE, &[1, 2, 3, 4])
        .unwrap();
    assert!(interpreter.end_poke().unwrap());
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![write(0xE002FE, 1, 1), write(0xE002FF, 1, 2)],
        "the bytes in the range, one write each"
    );
    assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);

    // a host write outside a Poke is a store into the range all the same, and a
    // Poke that changes nothing writes nothing
    interpreter.write_memory_bytes(0xE00100, &[0, 0]).unwrap();
    interpreter.begin_poke().unwrap();
    interpreter.write_memory_bytes(0xE00100, &[0, 0]).unwrap();
    assert!(!interpreter.end_poke().unwrap());
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![write(0xE00100, 1, 0), write(0xE00101, 1, 0)]
    );
    assert_eq!(interpreter.step().unwrap(), InterpreterStatus::Terminated);
}

#[test]
fn undo_rewinds_a_watched_store_and_empties_the_log() {
    let mut interpreter = watching(
        "    org $1000
    move.w #$1234,$E00100
    nop
",
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    interpreter.undo().unwrap();
    assert_eq!(bytes(&interpreter, 0xE00100, 2), vec![0xFF, 0xFF]);
    assert_eq!(interpreter.get_pc(), 0x1000);
    assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
    assert!(
        interpreter.take_watched_writes().is_empty(),
        "the write not taken was the one undone"
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::WatchedWrite);
    assert_eq!(
        interpreter.take_watched_writes(),
        vec![write(0xE00100, 2, 0x1234)],
        "and running it again records it again"
    );
}

#[test]
fn undoing_the_step_after_a_watched_write_goes_back_to_the_stop() {
    let mut interpreter = watching(
        "    org $1000
    move.b #1,$E00110
    move.l #7,d0
    nop
",
    );
    interpreter.step().unwrap();
    interpreter.take_watched_writes();
    assert_eq!(interpreter.step().unwrap(), InterpreterStatus::Running);
    interpreter.undo().unwrap();
    assert_eq!(*interpreter.get_status(), InterpreterStatus::WatchedWrite);
    assert_eq!(interpreter.get_pc(), 0x1004);
    assert!(interpreter.take_watched_writes().is_empty());
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
    assert_eq!(data(&interpreter, 0), 7);
}

// ---------------------------------------------------------------------------
// The count of executed instructions
// ---------------------------------------------------------------------------

#[test]
fn a_run_that_stops_early_still_counts_every_instruction_it_ran() {
    let mut interpreter = watching(
        "    org $1000
    moveq #1,d0
    moveq #2,d1
    moveq #3,d2
    move.b d0,$E00110
    moveq #4,d3
",
    );
    assert_eq!(interpreter.get_executed_instructions(), 0);
    assert_eq!(
        interpreter.run_with_limit(1000).unwrap(),
        InterpreterStatus::WatchedWrite
    );
    //three moveq and the store, though the run answered after one stop
    assert_eq!(interpreter.get_executed_instructions(), 4);
    interpreter.run_with_limit(1000).unwrap();
    assert_eq!(interpreter.get_executed_instructions(), 5);
}

#[test]
fn undo_does_not_take_executed_instructions_back() {
    let mut interpreter = watching(
        "    org $1000
    moveq #1,d0
    moveq #2,d1
",
    );
    interpreter.step().unwrap();
    interpreter.step().unwrap();
    interpreter.undo().unwrap();
    assert_eq!(interpreter.get_executed_instructions(), 2);
    interpreter.step().unwrap();
    assert_eq!(interpreter.get_executed_instructions(), 3);
}
