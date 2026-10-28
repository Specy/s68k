//! What the Interpreter does with the instructions only the decoder makes.
//!
//! The line A and line F words take vectors 10 and 11, and a word that is no
//! instruction is an illegal instruction, vector 4. None of them can reach the
//! Interpreter from source, so each is placed in a Program by hand. (`stop`
//! and `reset` were here until the plan's milestone C8 assembled and ran them;
//! `src/test/interrupts.rs` has them now.)

use crate::assembler::program::{AssembledInstruction, Program};
use crate::assembler::source::Location;
use crate::assembler::symbols::SymbolTable;
use crate::instructions::Instruction;
use crate::interpreter::{Interpreter, InterpreterStatus, RuntimeError, Termination};
use crate::machine::encode;

/// Runs a Program of `instruction` alone, at `$1000`, at the size and with the
/// bytes of its encoding.
fn run(instruction: Instruction) -> (Interpreter, RuntimeError) {
    let bytes = encode::encode_bytes(&instruction, 0x1000).expect("every case encodes");
    let program = Program::new(
        vec![AssembledInstruction {
            address: 0x1000,
            size: bytes.len(),
            bytes,
            instruction,
            location: Location::new("main.x68", 0, 4, 7),
            include_chain: Vec::new(),
            source: "    dc.w ...".to_string(),
        }],
        Vec::new(),
        &SymbolTable::new(),
        0x1000,
    );
    let mut interpreter = Interpreter::new(program, None);
    let error = interpreter
        .run()
        .expect_err("the instruction ends the program");
    assert_eq!(
        *interpreter.get_status(),
        InterpreterStatus::TerminatedWithException
    );
    assert_eq!(
        interpreter.get_termination(),
        Some(&Termination::Exception {
            cause: error.clone(),
            vector: crate::machine::exception::vector_of(&error),
        })
    );
    (interpreter, error)
}

#[test]
fn the_line_a_and_line_f_words_take_their_vectors() {
    let (interpreter, error) = run(Instruction::LINEA(0xA123));
    assert_eq!(error, RuntimeError::LineA(0xA123));
    assert_eq!(
        interpreter.get_termination(),
        Some(&Termination::Exception {
            cause: RuntimeError::LineA(0xA123),
            vector: Some(10),
        }),
        "vector 10 holds no handler"
    );
    let (_, error) = run(Instruction::LINEF(0xF000));
    assert_eq!(error, RuntimeError::LineF(0xF000));
}

#[test]
fn a_word_that_is_no_instruction_is_an_illegal_instruction() {
    let (interpreter, error) = run(Instruction::IllegalWord(0x4AFB));
    assert_eq!(error, RuntimeError::IllegalInstruction);
    assert_eq!(
        interpreter.get_pc(),
        0x1002,
        "as `illegal` stops past itself, a word on"
    );
}
