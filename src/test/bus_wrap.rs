//! The end of the 24-bit bus: an access that runs past `$FFFFFF` goes on at
//! `$000000`, as on a 68000, whose address bus has 24 lines, and as
//! EASy68K's `& ADDRMASK` reads it.
//!
//! Only a word or a long can straddle the end, and of those only a long at
//! `$FFFFFE` on an even address; a run of bytes can, and so can an
//! instruction, whose words wrap as it is fetched. Every way into memory is
//! checked: the program's reads and writes and the stack, undo, the decode
//! cache's fetch and the clearing of an instruction a write changes, the
//! memory map, the watched range, a trap task's answer and the host's reads
//! and writes.

use crate::assembler::program::{MemoryRangeKind, Program};
use crate::instructions::{InterruptResult, RegisterOperand, Size};
use crate::interpreter::{
    Access, Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError, WatchedWrite,
};

fn assemble(code: &str) -> Program {
    let assembly = crate::assembler::assemble_source(code);
    match assembly.program {
        Some(program) => program,
        None => panic!("the program does not assemble: {:?}", assembly.diagnostics),
    }
}

fn machine(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: true,
            history_size: 1000,
        }),
    )
}

fn d(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
}

fn label(interpreter: &Interpreter, name: &str) -> usize {
    interpreter
        .get_program()
        .symbols()
        .get(name)
        .unwrap_or_else(|| panic!("no label `{name}`"))
        .value as usize
}

fn bytes(interpreter: &Interpreter, address: usize, length: usize) -> Vec<u8> {
    interpreter
        .get_memory()
        .read_bytes(address, length)
        .unwrap()
        .to_vec()
}

#[test]
fn a_long_at_the_top_of_memory_wraps_round_to_the_bottom() {
    let mut interpreter = machine(
        "    org $1000
start:
    move.l #$11223344,$FFFFFE
    move.l $FFFFFE,d1
    move.l #$2,a7
    move.l #$55667788,-(a7)
    move.l (a7)+,d2
    simhalt
",
    );
    assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
    assert_eq!(d(&interpreter, 1), 0x1122_3344);
    assert_eq!(d(&interpreter, 2), 0x5566_7788);
    assert_eq!(bytes(&interpreter, 0xFFFFFE, 2), [0x55, 0x66]);
    assert_eq!(bytes(&interpreter, 0, 2), [0x77, 0x88]);
    assert_eq!(
        bytes(&interpreter, 0xFFFFFE, 4),
        [0x55, 0x66, 0x77, 0x88],
        "the host reads round the end too"
    );
    while interpreter.can_undo() {
        interpreter.undo().unwrap();
    }
    assert_eq!(
        bytes(&interpreter, 0xFFFFFC, 8),
        [0xFF; 8],
        "undo wraps as well"
    );
}

/// An instruction whose words run past the end is fetched round it, and a
/// write to `$000000` clears it from the decode cache, though no instruction
/// starts in that page: the jump below runs to the address its last word
/// holds, and runs to the new one once the program has changed it.
#[test]
fn an_instruction_that_wraps_round_runs_and_a_write_past_the_end_changes_it() {
    let mut interpreter = machine(
        "    org $1000
start:
    jmp $FFFFFC
first:
    addq.l #1,d1
    move.w #second,$0
    jmp $FFFFFC
second:
    addq.l #1,d2
    simhalt
",
    );
    let first = label(&interpreter, "first") as u16;
    let [high, low] = first.to_be_bytes();
    //`jmp first.l`, its low word at $000000
    interpreter
        .write_memory_bytes(0xFFFFFC, &[0x4E, 0xF9, 0x00, 0x00, high, low])
        .unwrap();
    assert_eq!(bytes(&interpreter, 0, 2), [high, low]);
    assert_eq!(
        interpreter.run_with_limit(20),
        Ok(InterpreterStatus::Paused)
    );
    assert_eq!((d(&interpreter, 1), d(&interpreter, 2)), (1, 1));
    assert_eq!(interpreter.get_pc(), label(&interpreter, "second") + 6);
}

#[test]
fn the_memory_map_reads_a_write_round_the_end() {
    let mut interpreter = machine(
        "    memory invalid $0,$3
    org $1000
start:
    move.l #$11223344,$FFFFFE
    simhalt
",
    );
    assert_eq!(
        interpreter.run(),
        Err(RuntimeError::BusError {
            address: 0xFFFFFE,
            size: Size::Long,
            access: Access::Write,
            range: MemoryRangeKind::Invalid,
        })
    );
    assert_eq!(bytes(&interpreter, 0xFFFFFE, 4), [0xFF; 4]);
}

#[test]
fn a_watched_range_at_the_bottom_sees_a_store_round_the_end() {
    let mut interpreter = machine(
        "    org $1000
start:
    move.l #$11223344,$FFFFFE
    move.l #$55667788,$FFFFFA
    move.w #$99AA,$2
    simhalt
",
    );
    interpreter.watch_memory(0, 2).unwrap();
    assert_eq!(interpreter.run(), Ok(InterpreterStatus::WatchedWrite));
    assert_eq!(
        interpreter.take_watched_writes(),
        [WatchedWrite {
            address: 0xFFFFFE,
            size: 4,
            value: 0x1122_3344
        }]
    );
    assert_eq!(
        interpreter.run(),
        Ok(InterpreterStatus::Paused),
        "nor the two after it"
    );
    assert!(interpreter.take_watched_writes().is_empty());
}

#[test]
fn a_line_answered_at_the_top_of_memory_wraps_round() {
    let mut interpreter = machine(
        "    org $1000
start:
    move.l #$FFFFFE,a1
    move.b #2,d0
    trap #15
    simhalt
",
    );
    assert_eq!(interpreter.run(), Ok(InterpreterStatus::Interrupt));
    interpreter
        .answer_interrupt(InterruptResult::ReadKeyboardString("abc".to_string()))
        .unwrap();
    assert_eq!(bytes(&interpreter, 0xFFFFFE, 2), b"ab");
    assert_eq!(bytes(&interpreter, 0, 2), b"c\0");
    interpreter.undo().unwrap();
    assert_eq!(bytes(&interpreter, 0xFFFFFE, 4), [0xFF; 4]);
}

#[test]
fn the_host_writes_and_reads_round_the_end_but_not_more_than_all_of_memory() {
    let mut interpreter = machine("    org $1000\nstart:\n    simhalt\n");
    interpreter
        .write_memory_bytes(0xFFFFFF, &[1, 2, 3])
        .unwrap();
    assert_eq!(bytes(&interpreter, 0xFFFFFF, 3), [1, 2, 3]);
    assert_eq!(bytes(&interpreter, 0, 2), [2, 3]);
    assert_eq!(
        interpreter.wasm_read_memory_bytes(0xFFFFFF, 3),
        [1, 2, 3],
        "what the editor's memory view reads"
    );
    assert!(interpreter
        .get_memory()
        .read_bytes(0x10, 0x100_0001)
        .is_err());
    assert!(interpreter
        .write_memory_bytes(0x10, &vec![0; 0x100_0001])
        .is_err());
}
