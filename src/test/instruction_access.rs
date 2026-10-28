//! Loads and stores that touch an instruction: the instructions are looked up
//! in the Program rather than run from memory, so the program is refused the
//! bytes under them, as MARS and RARS refuse a load or store in the text
//! segment. Their encodings are in memory all the same: the host reads them,
//! and is refused a write.

use crate::assembler::program::Program;
use crate::instructions::InterruptResult;
use crate::interpreter::{
    Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError, Termination,
};

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

fn interpreter(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: true,
            history_size: 100,
        }),
    )
}

/// What `code`, run to its end, ended with.
fn ending(code: &str) -> Termination {
    let mut interpreter = interpreter(code);
    let _ = interpreter.run();
    interpreter
        .get_termination()
        .cloned()
        .expect("the program ended")
}

fn refused(address: usize, write: bool) -> Termination {
    Termination::Exception(RuntimeError::InstructionAccess { address, write })
}

/// A program at `$1000`, `start` being its two-byte `nop`, the body from
/// `$1002`, then the four bytes of `simhalt` and a long, `data`.
fn at_start(body: &str) -> String {
    format!(
        "    org $1000\nstart:\n    nop\n{}\n    simhalt\ndata: dc.l 0\n    end start",
        body
    )
}

#[test]
fn every_size_of_load_from_an_instruction_is_refused() {
    for load in ["move.b start,d0", "move.w start,d0", "move.l start,d0"] {
        assert_eq!(ending(&at_start(load)), refused(0x1000, false), "{}", load);
    }
    assert_eq!(
        ending(&at_start("    move.b start+1,d0")),
        refused(0x1001, false),
        "the last byte of one"
    );
}

#[test]
fn every_size_of_store_to_an_instruction_is_refused() {
    for store in ["move.b d0,start", "move.w d0,start", "move.l d0,start"] {
        assert_eq!(ending(&at_start(store)), refused(0x1000, true), "{}", store);
    }
    // `clr.w` of an address a word holds is two words, at `$1002`
    assert_eq!(
        ending(&at_start("    clr.w start+4")),
        refused(0x1004, true),
        "the second word of one"
    );
}

#[test]
fn an_access_that_only_ends_in_an_instruction_is_refused() {
    // $ffe..$1001 starts before the first instruction and ends in it
    assert_eq!(
        ending(&at_start("    move.l $ffe,d0")),
        refused(0xffe, false)
    );
    // the long before `data` holds the last two bytes of simhalt: the body's
    // `move.l` of a label below is six bytes, so `simhalt` is at `$1008`
    assert_eq!(
        ending(&at_start("    move.l data-2,d0")),
        refused(0x100a, false)
    );
}

#[test]
fn data_beside_the_instructions_is_not() {
    let mut interpreter = interpreter(&at_start(
        "    move.l #$12345678,data\n    move.l data,d1\n    move.l $ffc,d2\n    move.l d1,$ffc",
    ));
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Paused);
    assert_eq!(
        interpreter.get_memory().read_long(0xffc).unwrap(),
        0x12345678
    );
}

#[test]
fn the_stack_and_a_program_with_no_instruction_before_it_are_not() {
    let mut interpreter = interpreter(&at_start("    move.l #1,-(a7)\n    move.l (a7)+,d0"));
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Paused);
}

#[test]
fn a_trap_task_reading_a_string_from_an_instruction_is_refused() {
    assert_eq!(
        ending(&at_start(
            "    lea start,a1\n    move.b #14,d0\n    trap #15"
        )),
        refused(0x1000, false)
    );
}

#[test]
fn an_answer_that_writes_over_an_instruction_is_refused() {
    let mut interpreter = interpreter(&at_start(
        "    lea start,a1\n    move.b #2,d0\n    trap #15",
    ));
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Interrupt);
    assert_eq!(
        interpreter.answer_interrupt(InterruptResult::ReadKeyboardString("hi".to_string())),
        Err(RuntimeError::InstructionAccess {
            address: 0x1000,
            write: true
        })
    );
}

#[test]
fn the_host_reads_an_instruction_and_may_not_write_one() {
    let mut interpreter = interpreter(&at_start(""));
    // `nop` and then `simhalt`, as a 68000 would hold them
    assert_eq!(
        interpreter.get_memory().read_bytes(0x1000, 6).unwrap(),
        [0x4E, 0x71, 0xFF, 0xFF, 0xFF, 0xFF]
    );
    assert_eq!(
        interpreter.write_memory_bytes(0xffe, &[1, 2, 3]),
        Err(RuntimeError::InstructionAccess {
            address: 0xffe,
            write: true
        })
    );
    assert!(interpreter.write_memory_bytes(0xffc, &[1, 2, 3, 4]).is_ok());
}

/// Every instruction's encoding is in memory where it was laid out, as its
/// bytes, and the pad before an aligned one keeps the fill.
#[test]
fn the_code_is_in_memory_as_its_encoding() {
    let program = assemble(
        "    org $1000
start:
    moveq #5,d0
    bra.s start
    move.l #$12345678,d1
    dc.b 7
    lea start,a0
    simhalt
",
    );
    let interpreter = Interpreter::new(program.clone(), None);
    let memory = interpreter.get_memory();
    assert_eq!(
        memory.read_bytes(0x1000, 0x16).unwrap(),
        [
            0x70, 0x05, // moveq #5,d0
            0x60, 0xFC, // bra.s back to $1000
            0x22, 0x3C, 0x12, 0x34, 0x56, 0x78, // move.l #$12345678,d1
            0x07, 0xFF, // the dc.b, and the pad byte before the lea, never written
            0x41, 0xF8, 0x10, 0x00, // lea $1000.w,a0
            0xFF, 0xFF, 0xFF, 0xFF, // simhalt
            0xFF, 0xFF, // and memory as it was
        ]
    );
    for instruction in program.instructions() {
        assert_eq!(
            memory
                .read_bytes(instruction.address, instruction.size)
                .unwrap(),
            instruction.bytes.as_slice(),
            "{}",
            instruction.source
        );
    }
}
