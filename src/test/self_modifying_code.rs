//! Code is memory: the Interpreter runs what memory holds, through the decode
//! cache, so a write into code runs as its new bytes decode (ADR 0006), and a
//! program ends where it runs into `$FFFF $FFFF` it never placed (ADR 0009).
//!
//! These replace the tests of `InstructionAccess`, the error a load or store
//! into code was while the Interpreter ran the Program's instruction list.

use crate::assembler::program::Program;
use crate::instructions::{Bytes, InterruptResult, RegisterOperand, Size};
use crate::interpreter::{
    CodeWrite, Interpreter, InterpreterOptions, InterpreterStatus, RunOptions, RuntimeError,
    Termination,
};
use crate::machine::decode::is_legal;
use crate::machine::Processor;

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

fn without_history(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: false,
            history_size: 0,
        }),
    )
}

fn data(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
}

fn label(interpreter: &Interpreter, name: &str) -> usize {
    interpreter.get_program().symbols()[name].value as usize
}

/// A `move.w` that writes `moveq #5,d0` over the `moveq #1,d0` after it.
const NEXT: &str = "    org $1000
start:
    move.w #$7005,next
next:
    moveq #1,d0
    nop
";

#[test]
fn a_write_into_the_next_instruction_runs_the_new_one() {
    let mut interpreter = interpreter(NEXT);
    let next = label(&interpreter, "next");
    assert_eq!(interpreter.step().unwrap(), InterpreterStatus::Running);
    let written = CodeWrite {
        address: next,
        instruction_index: Some(1),
    };
    assert_eq!(interpreter.get_code_write(), Some(written));
    assert_eq!(
        interpreter.get_last_steps(1)[0].get_code_write(),
        Some(written),
        "and the step says so"
    );
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
    assert_eq!(data(&interpreter, 0), 5, "the moveq the program wrote ran");
    assert_eq!(
        interpreter.get_code_write(),
        None,
        "the nop, the last instruction, wrote nothing"
    );
}

#[test]
fn a_changed_line_keeps_its_location_and_disassembles_as_it_is_now() {
    let mut interpreter = interpreter(NEXT);
    let next = label(&interpreter, "next");
    interpreter.step().unwrap();
    assert_eq!(interpreter.get_pc(), next);
    let line = interpreter.get_current_location().map(|location| location.line);
    assert_eq!(line, Some(4), "the moveq's line, which the source still says");
    let assembled = interpreter.get_instruction_at(next).unwrap();
    assert_eq!(assembled.bytes, [0x70, 0x01], "the Program keeps what it assembled");
    let now = interpreter.disassemble_at(next);
    assert_eq!(now.text, "MOVEQ #5,D0");
    assert_eq!(now.bytes, [0x70, 0x05]);
    assert_eq!((now.address as usize, now.size), (next, 2));
    interpreter.step().unwrap();
    assert_eq!(
        interpreter.get_last_steps(1)[0]
            .get_location()
            .map(|location| location.line),
        Some(4),
        "the step that ran the new bytes is on that line too"
    );
}

#[test]
fn undo_puts_the_old_instruction_back_and_it_runs_again() {
    // the addq at `start` runs, is overwritten with `addq.l #5,d0`, and runs
    // again as that
    let mut interpreter = interpreter(
        "    org $1000
start:
    addq.l #1,d0
    move.w #$5A80,start
    bra start
",
    );
    for _ in 0..4 {
        interpreter.step().unwrap();
    }
    assert_eq!(data(&interpreter, 0), 6, "1, then 5 more");
    for _ in 0..4 {
        interpreter.undo().unwrap();
    }
    assert_eq!(interpreter.get_pc(), 0x1000);
    assert_eq!(data(&interpreter, 0), 0);
    assert_eq!(
        &*interpreter.get_memory().read_bytes(0x1000, 2).unwrap(),
        [0x52, 0x80],
        "undo wrote the old bytes back"
    );
    assert_eq!(
        interpreter.get_code_write(),
        None,
        "and took the code write with it"
    );
    interpreter.step().unwrap();
    assert_eq!(data(&interpreter, 0), 1, "the old addq runs again");
    for _ in 0..3 {
        interpreter.step().unwrap();
    }
    assert_eq!(data(&interpreter, 0), 6, "and the new one after the write");
}

#[test]
fn a_poke_into_code_runs_and_is_no_code_write() {
    let mut interpreter = interpreter(
        "    org $1000
start:
    moveq #1,d0
    nop
",
    );
    interpreter.begin_poke().unwrap();
    interpreter.write_memory_bytes(0x1000, &[0x70, 0x07]).unwrap();
    assert!(interpreter.end_poke().unwrap());
    assert_eq!(interpreter.get_code_write(), None);
    assert_eq!(interpreter.get_last_steps(1)[0].get_code_write(), None);
    interpreter.step().unwrap();
    assert_eq!(data(&interpreter, 0), 7, "the poked moveq ran");
    assert_eq!(interpreter.get_code_write(), None);
    // undoing the moveq and the Poke puts the assembled one back
    interpreter.undo().unwrap();
    interpreter.undo().unwrap();
    interpreter.step().unwrap();
    assert_eq!(data(&interpreter, 0), 1);
}

#[test]
fn a_testcase_preset_over_code_runs_too() {
    // the host's direct write, outside a Poke, which records nothing
    let mut interpreter = without_history(
        "    org $1000
start:
    moveq #1,d0
",
    );
    interpreter.write_memory_bytes(0x1000, &[0x70, 0x03]).unwrap();
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
    assert_eq!(data(&interpreter, 0), 3);
    assert_eq!(interpreter.get_code_write(), None);
}

#[test]
fn a_run_asked_to_stops_after_the_write_into_code() {
    let mut interpreter = interpreter(NEXT);
    let next = label(&interpreter, "next");
    let stopping = RunOptions::CONTINUE.stopping_on_code_writes();
    assert_eq!(
        interpreter.run_with_breakpoints(&[], None, stopping).unwrap(),
        InterpreterStatus::CodeWrite
    );
    assert_eq!(interpreter.get_pc(), next, "after the write, before what it wrote");
    assert_eq!(data(&interpreter, 0), 0);
    assert_eq!(
        interpreter.get_code_write(),
        Some(CodeWrite {
            address: next,
            instruction_index: Some(1)
        })
    );
    assert_eq!(
        interpreter.run_with_breakpoints(&[], None, stopping).unwrap(),
        InterpreterStatus::Terminated,
        "running again goes on, and stops for no other write"
    );
    assert_eq!(data(&interpreter, 0), 5);
}

#[test]
fn a_run_not_asked_to_does_not_stop() {
    let runs: [fn(&mut Interpreter) -> Result<InterpreterStatus, RuntimeError>; 3] = [
        |interpreter| interpreter.run(),
        |interpreter| interpreter.run_with_breakpoints(&[], None, RunOptions::CONTINUE),
        |interpreter| interpreter.run_with_limit(100),
    ];
    for run in runs {
        let mut interpreter = interpreter(NEXT);
        assert_eq!(run(&mut interpreter).unwrap(), InterpreterStatus::Terminated);
        assert_eq!(data(&interpreter, 0), 5);
    }
    let mut stepped = interpreter(NEXT);
    assert_eq!(stepped.step().unwrap(), InterpreterStatus::Running, "nor a step");
}

#[test]
fn a_code_write_by_the_last_instruction_stops_before_the_end() {
    // as a watched write does: the next call ends the program
    let mut interpreter = interpreter(
        "    org $1000
start:
    lea start,a0
    move.w d0,(a0)
",
    );
    let stopping = RunOptions::RESUME.stopping_on_code_writes();
    assert_eq!(
        interpreter.run_with_breakpoints(&[], Some(10), stopping).unwrap(),
        InterpreterStatus::CodeWrite
    );
    assert!(interpreter.has_reached_bottom());
    assert!(!interpreter.has_terminated());
    assert_eq!(
        interpreter.run_with_breakpoints(&[], Some(10), stopping).unwrap(),
        InterpreterStatus::Terminated
    );
    assert_eq!(
        interpreter.get_termination(),
        Some(&Termination::EndOfProgram { address: 0x1006 })
    );
}

#[test]
fn a_watched_write_into_code_is_answered_as_the_watched_write() {
    let mut interpreter = interpreter(NEXT);
    let next = label(&interpreter, "next");
    interpreter.watch_memory(next, next + 2).unwrap();
    let stopping = RunOptions::CONTINUE.stopping_on_code_writes();
    assert_eq!(
        interpreter.run_with_breakpoints(&[], None, stopping).unwrap(),
        InterpreterStatus::WatchedWrite
    );
    assert!(
        interpreter.get_code_write().is_some(),
        "and the code write is recorded"
    );
    assert_eq!(interpreter.take_watched_writes().len(), 1);
}

#[test]
fn undoing_a_code_write_stop_brings_back_the_status_before_it() {
    let mut interpreter = interpreter(NEXT);
    let stopping = RunOptions::CONTINUE.stopping_on_code_writes();
    interpreter.run_with_breakpoints(&[], None, stopping).unwrap();
    interpreter.undo().unwrap();
    assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
    assert_eq!(interpreter.get_pc(), 0x1000);
}

#[test]
fn the_code_write_status_is_appended_to_the_public_values() {
    assert_eq!(InterpreterStatus::WatchedWrite as u32, 5);
    assert_eq!(InterpreterStatus::CodeWrite as u32, 6);
}

#[test]
fn a_trap_task_buffer_written_over_code_invalidates_it() {
    // task 53 reads two bytes from file 1 into (A1), which is the nop
    let mut interpreter = interpreter(
        "    org $1000
start:
    lea next,a1
    move.l #1,d1
    move.l #2,d2
    move.b #53,d0
    trap #15
next:
    nop
    nop
",
    );
    let next = label(&interpreter, "next");
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Interrupt);
    interpreter
        .answer_interrupt(InterruptResult::ReadFile(Some(Bytes(vec![0x70, 0x07]))))
        .unwrap();
    let written = CodeWrite {
        address: next,
        instruction_index: Some(5),
    };
    assert_eq!(
        interpreter.get_code_write(),
        Some(written),
        "the trap's, for its answer"
    );
    assert_eq!(interpreter.get_last_steps(1)[0].get_code_write(), Some(written));
    assert_eq!(data(&interpreter, 0) & 0xFFFF, 0, "the read succeeded");
    assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
    assert_eq!(data(&interpreter, 0), 7, "the bytes read ran as moveq #7,d0");
}

#[test]
fn a_write_over_code_reached_in_data_names_no_line() {
    // `data` runs as code, `moveq #9,d0` and `rts`, then is written over and
    // runs again as `moveq #3,d0`
    let mut interpreter = interpreter(
        "    org $1000
start:
    jsr data
    move.w #$7003,data
    jsr data
    move.b #9,d0
    trap #15
data:
    dc.w $7009,$4E75
",
    );
    let data_address = label(&interpreter, "data");
    interpreter.step().unwrap();
    interpreter.step().unwrap();
    interpreter.step().unwrap();
    assert_eq!(data(&interpreter, 0), 9);
    assert_eq!(
        interpreter.get_last_steps(1)[0].get_location(),
        None,
        "the rts in the data is on no line"
    );
    interpreter.step().unwrap();
    assert_eq!(
        interpreter.get_code_write(),
        Some(CodeWrite {
            address: data_address,
            instruction_index: None
        }),
        "the data that ran is code, and no line of the Program"
    );
    for _ in 0..3 {
        interpreter.step().unwrap();
    }
    assert_eq!(data(&interpreter, 0), 3);
}

#[test]
fn a_jump_into_data_runs_what_its_words_decode_to() {
    let illegal = (0..=u16::MAX)
        .find(|word| !is_legal(*word, Processor::M68000))
        .expect("a word that is no instruction");
    let mut interpreter = interpreter(&format!(
        "    org $1000
start:
    jmp data
data:
    dc.w $7009,${illegal:04X}
"
    ));
    let data_address = label(&interpreter, "data");
    interpreter.step().unwrap();
    assert_eq!(interpreter.get_pc(), data_address);
    assert_eq!(
        interpreter.get_current_location(),
        None,
        "no line was laid out there"
    );
    assert_eq!(interpreter.disassemble_at(data_address).text, "MOVEQ #9,D0");
    assert_eq!(
        interpreter.disassemble_at(data_address + 2).text,
        format!("DC.W ${illegal:04X}")
    );
    interpreter.step().unwrap();
    assert_eq!(data(&interpreter, 0), 9);
    assert_eq!(interpreter.step(), Err(RuntimeError::IllegalInstruction));
    assert_eq!(
        *interpreter.get_status(),
        InterpreterStatus::TerminatedWithException
    );
}

/// Memory decodes for the Program's processor (ADR 0011): `$42C0` is
/// `move ccr,d0` to a 68010 Program and a word that is no instruction to a
/// 68000 one, in the disassembly and when it runs.
#[test]
fn a_decode_reads_the_words_for_the_programs_processor() {
    let source = |processor: &str| {
        format!(
            "    opt p={processor}
    org $1000
start:
    move.w #$1f,ccr
    jmp data
data:
    dc.w $42C0,$49C1
"
        )
    };
    let expected = [
        ("68000", "DC.W $42C0", "DC.W $49C1"),
        ("68010", "MOVE CCR,D0", "DC.W $49C1"),
        ("68020", "MOVE CCR,D0", "EXTB.L D1"),
    ];
    for (processor, first, second) in expected {
        let mut interpreter = interpreter(&source(processor));
        let data_address = label(&interpreter, "data");
        assert_eq!(
            interpreter.disassemble_at(data_address).text,
            first,
            "{processor}"
        );
        assert_eq!(
            interpreter.disassemble_at(data_address + 2).text,
            second,
            "{processor}"
        );
        interpreter.step().unwrap();
        interpreter.step().unwrap();
        match processor {
            "68000" => assert_eq!(interpreter.step(), Err(RuntimeError::IllegalInstruction)),
            _ => {
                interpreter.step().unwrap();
                assert_eq!(
                    data(&interpreter, 0),
                    0x1f,
                    "the decode miss ran `move ccr,d0`"
                );
            }
        }
    }
}

#[test]
fn a_jump_to_an_odd_address_is_an_address_error_of_the_jump() {
    let mut interpreter = interpreter(
        "    org $1000
start:
    lea 1(pc),a0
    jmp (a0)
",
    );
    interpreter.step().unwrap();
    let odd = interpreter.get_register_value(RegisterOperand::Address(0), Size::Long) as usize;
    assert_eq!(odd & 1, 1);
    // the 68000 prefetches from the target in the jump itself, so the jump is
    // what faults, and vector 3 holds no handler
    assert_eq!(
        interpreter.step(),
        Err(RuntimeError::AddressError {
            address: odd,
            size: Size::Word,
            access: crate::interpreter::Access::Program,
        })
    );
    // the step that failed is undone, and the program is back on the jump
    interpreter.undo().unwrap();
    assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
    assert_eq!(interpreter.get_pc(), 0x1004);
}

mod the_end {
    use super::*;

    #[test]
    fn running_off_the_last_instruction_ends_the_program_there() {
        let mut interpreter = interpreter("    org $1000\n    nop\n    nop\n");
        assert!(!interpreter.has_reached_bottom());
        assert_eq!(interpreter.step().unwrap(), InterpreterStatus::Running);
        assert_eq!(
            interpreter.step().unwrap(),
            InterpreterStatus::Terminated,
            "in the step that ran the last instruction"
        );
        assert!(interpreter.has_reached_bottom());
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::EndOfProgram { address: 0x1004 })
        );
        interpreter.undo().unwrap();
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
        assert_eq!(
            interpreter.get_termination(),
            None,
            "undo takes the end back"
        );
    }

    #[test]
    fn a_halt_the_program_placed_pauses() {
        // `$FFFF $FFFF` in the program's own data is a simhalt, as in EASy68K
        let mut interpreter = interpreter(
            "    org $1000
start:
    jmp halt
    nop
halt:
    dc.w $FFFF,$FFFF
    moveq #4,d0
",
        );
        assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Paused);
        assert!(!interpreter.has_reached_bottom());
        assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
        assert_eq!(data(&interpreter, 0), 4, "and resumes after it");
    }

    #[test]
    fn a_halt_that_straddles_the_placed_bytes_is_the_programs() {
        // the last word of the data is `$FFFF` and the next is memory's fill:
        // two of the four bytes are placed, so it pauses
        let mut interpreter = interpreter(
            "    org $1000
start:
    jmp halt
halt:
    dc.w $FFFF
",
        );
        assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Paused);
    }

    #[test]
    fn a_halt_the_program_did_not_place_ends_it() {
        // the program writes `$FFFF $FFFF` into memory it never placed and jumps
        // there: the bytes are the program's at run time, not placed, and end it
        let mut interpreter = interpreter(
            "    org $1000
start:
    lea $3000,a0
    move.l #$12345678,(a0)
    move.l #$FFFFFFFF,(a0)
    jmp (a0)
",
        );
        assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::EndOfProgram { address: 0x3000 })
        );
    }

    #[test]
    fn a_jump_into_memory_the_program_never_placed_ends_it() {
        let mut interpreter = interpreter(
            "    org $1000
start:
    jmp $5000
",
        );
        assert_eq!(interpreter.step().unwrap(), InterpreterStatus::Terminated);
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::EndOfProgram { address: 0x5000 })
        );
    }

    #[test]
    fn running_on_past_the_code_runs_the_data_after_it() {
        // `$7009` is `moveq #9,d0`, and the program ends after it
        let mut interpreter = interpreter(
            "    org $1000
start:
    moveq #1,d0
    dc.w $7009
",
        );
        assert_eq!(interpreter.run().unwrap(), InterpreterStatus::Terminated);
        assert_eq!(data(&interpreter, 0), 9);
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::EndOfProgram { address: 0x1004 })
        );
    }

    #[test]
    fn a_program_whose_entry_point_holds_nothing_has_ended_before_it_starts() {
        let interpreter = interpreter(
            "    org $1000
    nop
    end $2000
",
        );
        assert!(interpreter.has_terminated());
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::EndOfProgram { address: 0x2000 })
        );
    }
}

#[test]
fn the_cache_takes_a_page_for_the_code_and_none_for_the_stack() {
    let mut interpreter = without_history(
        "    org $1000
start:
    move.l #1,-(a7)
    move.l (a7)+,d0
    lea $20000,a0
    move.l d0,(a0)
",
    );
    assert_eq!(interpreter.get_memory().cache().pages_in_use(), 1);
    interpreter.run().unwrap();
    assert_eq!(
        interpreter.get_memory().cache().pages_in_use(),
        1,
        "the stack, the data and the end, in memory nothing placed, are no page"
    );
}

#[test]
fn the_cache_has_a_page_for_every_4_kb_of_code() {
    // 4,000 two-byte instructions from $1000 are 8,000 bytes, in two pages
    let code = format!("    org $1000\nstart:\n{}", "    nop\n".repeat(4000));
    let interpreter = without_history(&code);
    assert_eq!(interpreter.get_memory().cache().pages_in_use(), 2);
}

/// A Program with no instruction line runs what it placed at its Entry point,
/// as EASy68K runs the words of an S-record from its start address; an empty
/// one, whose Entry point is on memory it never placed, ends before it starts
/// (batch 1 review, unverified items).
#[test]
fn a_program_of_data_words_runs_from_its_entry_point() {
    let mut words = interpreter(
        "    org $1000
start:
    dc.w $7005
    dc.w $4e71
    end start
",
    );
    assert!(!words.has_terminated(), "it has words to run");
    words.run().unwrap();
    assert_eq!(words.get_register_value(RegisterOperand::Data(0), Size::Long), 5);
    assert_eq!(
        words.get_termination(),
        Some(&Termination::EndOfProgram { address: 0x1004 })
    );
    let empty = interpreter("* Nothing but a comment.\n");
    assert_eq!(
        empty.get_termination(),
        Some(&Termination::EndOfProgram { address: 0x1000 })
    );
}
