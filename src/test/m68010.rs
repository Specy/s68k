//! The 68010 dialect (ADR 0011; the asm-editor's `docs/design/m68k-machine.md`,
//! decision 3): its instructions, `move sr,<ea>` made privileged, the vector
//! base register, the format `$0` and `$8` frames and `rte`'s check of them.
//!
//! Every program here is written for the 68010 with `opt p=68010` but where a
//! test sets the 68000 beside it, and installs its handlers by writing the
//! vectors where VBR says the table is. Undo is checked across all of it, VBR,
//! SFC and DFC included.

use crate::assembler::program::Program;
use crate::debugger::SystemRegister;
use crate::instructions::{RegisterOperand, Size};
use crate::interpreter::{
    Access, Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError, Termination,
};

fn assemble(code: &str) -> Program {
    let assembly = crate::assembler::assemble_source(code);
    match assembly.program {
        Some(program) => program,
        None => panic!(
            "the program does not assemble: {:?}",
            assembly
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .map(|diagnostic| format!(
                    "line {}: {}",
                    diagnostic.location.line + 1,
                    diagnostic.message()
                ))
                .collect::<Vec<_>>()
        ),
    }
}

/// An Interpreter for `body`, written for the 68010, at `$1000`.
fn m68010(body: &str) -> Interpreter {
    machine(&format!("    opt p=68010\n    org $1000\nstart:\n{body}"))
}

fn machine(code: &str) -> Interpreter {
    Interpreter::new(
        assemble(code),
        Some(InterpreterOptions {
            keep_history: true,
            history_size: 10_000,
        }),
    )
}

fn d(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
}

fn a(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Address(register), Size::Long)
}

fn label(interpreter: &Interpreter, name: &str) -> u32 {
    interpreter
        .get_program()
        .symbols()
        .get(name)
        .unwrap_or_else(|| panic!("no label `{name}`"))
        .value as u32
}

fn word(interpreter: &Interpreter, address: u32) -> u16 {
    interpreter
        .get_memory()
        .read_word(address as usize)
        .unwrap()
}

fn long(interpreter: &Interpreter, address: u32) -> u32 {
    interpreter
        .get_memory()
        .read_long(address as usize)
        .unwrap()
}

/// Runs to the `simhalt` that ends every program here.
fn run_to_the_halt(interpreter: &mut Interpreter) {
    assert_eq!(
        interpreter.run(),
        Ok(InterpreterStatus::Paused),
        "{:?}",
        interpreter.get_termination()
    );
}

/// Everything the programs here change, to compare across undo.
#[derive(Debug, PartialEq)]
struct Snapshot {
    registers: Vec<u32>,
    system: crate::interpreter::SystemRegisters,
    pc: usize,
    status: InterpreterStatus,
    termination: Option<Termination>,
    memory: Vec<u8>,
}

fn snapshot(interpreter: &Interpreter) -> Snapshot {
    let memory = interpreter.get_memory();
    let mut bytes = Vec::new();
    for (start, end) in [
        (0x0, 0x400),
        (0x1000, 0x1400),
        (0x8000, 0x8400),
        (0xFFFF00, 0x1000000),
    ] {
        bytes.extend_from_slice(&memory.read_bytes(start, end - start).unwrap());
    }
    Snapshot {
        registers: interpreter.get_cpu().get_register_values(),
        system: interpreter.get_system(),
        pc: interpreter.get_pc(),
        status: *interpreter.get_status(),
        termination: interpreter.get_termination().cloned(),
        memory: bytes,
    }
}

/// Steps `interpreter` to its end, a snapshot before every step, then undoes
/// every step and checks that each undo puts back the snapshot taken before
/// the step it undid.
fn undo_restores_every_step(mut interpreter: Interpreter) {
    let mut snapshots = vec![snapshot(&interpreter)];
    loop {
        let stepped = interpreter.step();
        snapshots.push(snapshot(&interpreter));
        match stepped {
            Ok(InterpreterStatus::Running) => continue,
            _ => break,
        }
    }
    snapshots.pop();
    while let Some(expected) = snapshots.pop() {
        if snapshots.is_empty() {
            break;
        }
        interpreter.undo().unwrap();
        assert_eq!(snapshot(&interpreter), expected);
    }
}

mod control_registers {
    use super::*;

    #[test]
    fn movec_writes_and_reads_vbr_sfc_dfc_and_usp() {
        let mut interpreter = m68010(
            "    move.l #$8000,d0
    movec d0,vbr
    movec vbr,d1
    moveq #-1,d0
    movec d0,sfc
    movec sfc,d2
    moveq #5,d0
    movec d0,dfc
    movec dfc,a3
    move.l #$4000,a0
    movec a0,usp
    movec usp,d4
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 1), 0x8000);
        assert_eq!(d(&interpreter, 2), 7, "a function code is three bits");
        assert_eq!(a(&interpreter, 3), 5);
        assert_eq!(d(&interpreter, 4), 0x4000);
        let system = interpreter.get_system();
        assert_eq!(
            (system.vbr, system.sfc, system.dfc, system.usp),
            (Some(0x8000), Some(7), Some(5), 0x4000)
        );
        undo_restores_every_step(m68010(
            "    move.l #$8000,d0\n    movec d0,vbr\n    moveq #3,d0\n    movec d0,sfc\n    \
             movec d0,dfc\n    simhalt",
        ));
    }

    #[test]
    fn a_control_register_the_68010_does_not_have_is_an_illegal_instruction() {
        //`movec cacr,d0`, the 68020's, by its words
        let mut interpreter = m68010(
            "    move.l #illegal,$10
    dc.w $4E7A,$0002
    simhalt
illegal:
    move.l 2(a7),d7
    simhalt",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 7), label(&interpreter, "start") + 8);
        //and with no handler, the program ends as an illegal instruction does
        let mut interpreter = m68010("    dc.w $4E7B,$0802\n    simhalt");
        assert_eq!(interpreter.run(), Err(RuntimeError::IllegalInstruction));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: RuntimeError::IllegalInstruction,
                vector: Some(4)
            })
        );
    }

    #[test]
    fn movec_and_moves_are_privileged() {
        for line in [
            "    movec vbr,d0",
            "    movec d0,sfc",
            "    moves.w (a0),d0",
        ] {
            let mut interpreter = m68010(&format!(
                "    lea $8000,a0\n    andi #$DFFF,sr\n{line}\n    simhalt"
            ));
            assert_eq!(
                interpreter.run(),
                Err(RuntimeError::PrivilegeViolation),
                "{line}"
            );
            assert_eq!(
                interpreter.get_termination(),
                Some(&Termination::Exception {
                    cause: RuntimeError::PrivilegeViolation,
                    vector: Some(8)
                }),
                "{line}"
            );
        }
    }

    #[test]
    fn the_host_pokes_them_and_a_68000_has_none() {
        let mut interpreter = m68010("    nop\n    simhalt");
        interpreter.begin_poke().unwrap();
        interpreter
            .set_control_register(SystemRegister::Vbr, 0x10000)
            .unwrap();
        interpreter
            .set_control_register(SystemRegister::Dfc, 0xFE)
            .unwrap();
        assert!(interpreter.end_poke().unwrap());
        let system = interpreter.get_system();
        assert_eq!((system.vbr, system.dfc), (Some(0x10000), Some(6)));
        interpreter.undo().unwrap();
        let system = interpreter.get_system();
        assert_eq!((system.vbr, system.dfc), (Some(0), Some(0)));
        //outside a Poke the write is direct
        interpreter
            .set_control_register(SystemRegister::Sfc, 2)
            .unwrap();
        assert_eq!(interpreter.get_system().sfc, Some(2));
        assert!(matches!(
            interpreter.set_control_register(SystemRegister::Sr, 0),
            Err(RuntimeError::InvalidArgument(_))
        ));

        let mut interpreter = machine("    org $1000\n    nop\n    simhalt");
        let system = interpreter.get_system();
        assert_eq!((system.vbr, system.sfc, system.dfc), (None, None, None));
        let json = serde_json::to_value(system).unwrap();
        assert!(json.get("vbr").is_none(), "a 68000 has no VBR: {json}");
        assert!(matches!(
            interpreter.set_control_register(SystemRegister::Vbr, 0x8000),
            Err(RuntimeError::InvalidArgument(_))
        ));
    }
}

mod the_vector_base {
    use super::*;

    #[test]
    fn every_exception_reads_its_vector_through_vbr() {
        let mut interpreter = m68010(
            "    move.l #$8000,d0
    movec d0,vbr
    move.l #zero,$8000+5*4
    move.l #irq,$8000+26*4
    moveq #0,d1
    divu d1,d0
    simhalt
    simhalt
zero:
    moveq #5,d6
    rte
irq:
    moveq #2,d7
    rte",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 6), 5, "the handler VBR's table holds");
        assert_eq!(
            long(&interpreter, 5 * 4),
            0xFFFF_FFFF,
            "the table at $0 untouched"
        );
        //an IRQ's autovector too
        interpreter.raise_irq(2).unwrap();
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 7), 2);
    }

    #[test]
    fn a_vector_of_the_moved_table_with_no_handler_ends_the_program() {
        let mut interpreter = m68010(
            "    move.l #zero,$14
    move.l #$8000,d0
    movec d0,vbr
    divu #0,d0
    simhalt
zero:
    rte",
        );
        assert_eq!(interpreter.run(), Err(RuntimeError::DivisionByZero));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: RuntimeError::DivisionByZero,
                vector: Some(5)
            })
        );
    }
}

mod frames {
    use super::*;

    /// Groups 1 and 2 push the format `$0` frame: the status register, the
    /// program counter and the format/vector-offset word, eight bytes.
    #[test]
    fn a_trap_pushes_the_four_words_of_format_0() {
        let mut interpreter = m68010(
            "    move.l #handler,$80+3*4
    move.l a7,a6
    move #$2004,ccr
    trap #3
back:
    simhalt
handler:
    move.w (a7),d0
    move.l 2(a7),d1
    move.w 6(a7),d2
    move.l a7,d3
    rte",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 0x2004, "the status register");
        assert_eq!(d(&interpreter, 1), label(&interpreter, "back"));
        assert_eq!(d(&interpreter, 2), 0x008C, "format 0, vector 35 times four");
        assert_eq!(d(&interpreter, 3), a(&interpreter, 6) - 8, "eight bytes");
        assert_eq!(
            a(&interpreter, 7),
            a(&interpreter, 6),
            "`rte` pops all eight"
        );
        undo_restores_every_step(m68010(
            "    move.l #handler,$80+3*4\n    trap #3\n    simhalt\nhandler:\n    rte",
        ));
    }

    /// The 68010's frame of an address error is format `$8`, 29 words with
    /// three not written, and stacks the instruction that faulted: the
    /// handler mends the address, and `rte` runs the instruction again.
    #[test]
    fn an_address_error_pushes_format_8_and_rte_runs_the_instruction_again() {
        let mut interpreter = m68010(
            "    move.l #handler,$C
    lea $FFFF00,a7
    move.l #$12345678,d4
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    move.l d4,-(a7)
    lea $FFFF00,a7
    lea data+1,a0
    move.w #$4321,data
faulty:
    move.w (a0),d0
    simhalt
handler:
    move.l a7,a5
    subq.l #1,a0
    rte
data:
    dc.w 0",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 0x4321, "run again, from the mended a0");
        let frame = a(&interpreter, 5);
        assert_eq!(frame, 0xFFFF00 - 58, "29 words");
        let at = |offset: u32| word(&interpreter, frame + offset);
        assert_eq!(at(0), 0x2000, "the status register");
        assert_eq!(long(&interpreter, frame + 2), label(&interpreter, "faulty"));
        assert_eq!(at(6), 0x800C, "format 8, vector 3 times four");
        assert_eq!(
            at(8),
            0x1000 | 0x0100 | 5,
            "a data read, of a word, in supervisor data space"
        );
        assert_eq!(
            long(&interpreter, frame + 10),
            label(&interpreter, "data") + 1
        );
        //the three reserved words keep what the stack held, the high word of a long of d4
        for (offset, written) in [
            (14, false),
            (16, true),
            (18, false),
            (20, true),
            (22, false),
        ] {
            let expected = if written { 0 } else { 0x1234 };
            assert_eq!(at(offset), expected, "the word at +{offset}");
        }
        for offset in (24..58).step_by(2) {
            assert_eq!(at(offset), 0, "the internal word at +{offset}");
        }
        assert_eq!(a(&interpreter, 7), 0xFFFF00, "`rte` pops all 58 bytes");
    }

    /// A bus error's frame says what the access was: `clr` on a 68010 writes
    /// without reading, so the access it faults on is a write, where the
    /// 68000's `clr` faults on its read.
    #[test]
    fn a_bus_error_frame_says_the_68010s_clr_only_writes() {
        let program = |processor: &str| {
            format!(
                "    opt p={processor}
    memory invalid $9000,$9FFF
    org $1000
start:
    move.l #handler,$8
    clr.w $9000
    simhalt
handler:
    move.l a7,a5
    rte"
            )
        };
        let mut interpreter = machine(&program("68010"));
        interpreter.step().unwrap();
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        let frame = interpreter.get_ssp();
        assert_eq!(word(&interpreter, frame + 6), 0x8008, "format 8, vector 2");
        assert_eq!(word(&interpreter, frame + 8), 5, "a write, of a word");
        let mut interpreter = machine(&program("68000"));
        interpreter.step().unwrap();
        interpreter.step().unwrap();
        let frame = interpreter.get_ssp();
        assert_eq!(
            word(&interpreter, frame) & 0x1F,
            0x15,
            "the 68000's status word: a read in supervisor data space"
        );
    }
}

mod return_from_exception {
    use super::*;

    /// A frame of a format the 68010 does not have is a format error, vector
    /// 14: the frame is left where it is, and the frame of the format error
    /// stacks the `rte`, with N the format's high bit and Z and V cleared.
    #[test]
    fn rte_of_an_unknown_format_is_a_format_error() {
        for (format, negative) in [(0x1u16, false), (0x9, true)] {
            let mut interpreter = m68010(&format!(
                "    move.l #handler,$80
    move.l #format,$38
    trap #0
    simhalt
handler:
    move.w #${:X}080,6(a7)
    move.l a7,a6
    move #$07,ccr
bad:
    rte
format:
    move.w (a7),d0
    move.l 2(a7),d1
    move.w 6(a7),d2
    move.l a7,d3
    simhalt",
                format
            ));
            run_to_the_halt(&mut interpreter);
            assert_eq!(d(&interpreter, 1), label(&interpreter, "bad"), "the `rte`");
            assert_eq!(d(&interpreter, 2), 0x0038, "format 0, vector 14");
            assert_eq!(d(&interpreter, 3), a(&interpreter, 6) - 8, "over the frame");
            let flags = d(&interpreter, 0) & 0x0F;
            assert_eq!(
                flags,
                if negative { 0x9 } else { 0x1 },
                "format ${format:X}"
            );
        }
    }

    /// A format `$8` frame whose version number is not 0, the one s68k
    /// writes, is a format error too, with no change of flags.
    #[test]
    fn rte_of_a_format_8_frame_of_another_version_is_a_format_error() {
        let mut interpreter = m68010(
            "    move.l #handler,$C
    move.l #format,$38
    move.l #1,a0
    move.w (a0),d0
    simhalt
handler:
    move.w #$0400,26(a7)
    rte
format:
    simhalt",
        );
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
        //no handler for vector 14 ends the program with the format error
        let mut interpreter = m68010(
            "    move.l #handler,$C\n    move.l #1,a0\n    move.w (a0),d0\n    simhalt\n\
             handler:\n    move.w #$0400,26(a7)\n    rte",
        );
        assert_eq!(
            interpreter.run(),
            Err(RuntimeError::FormatError { format: 8 })
        );
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: RuntimeError::FormatError { format: 8 },
                vector: Some(14)
            })
        );
    }

    #[test]
    fn undo_puts_back_a_format_error() {
        undo_restores_every_step(m68010(
            "    move.l #handler,$80
    move.l #format,$38
    trap #0
    simhalt
handler:
    move.w #$5080,6(a7)
    rte
format:
    simhalt",
        ));
    }
}

mod instructions {
    use super::*;

    #[test]
    fn move_from_sr_is_privileged_from_the_68010_on() {
        let program = |processor: &str| {
            format!(
                "    opt p={processor}\n    org $1000\nstart:\n    andi #$DFFF,sr\n    \
                 move sr,d0\n    move ccr,d1\n    simhalt"
            )
        };
        let mut interpreter = machine(&program("68010"));
        assert_eq!(interpreter.run(), Err(RuntimeError::PrivilegeViolation));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: RuntimeError::PrivilegeViolation,
                vector: Some(8)
            })
        );
        //`move ccr,<ea>` is the 68010's for user mode, and a 68000 reads `sr` there
        let mut interpreter = machine(&program("68010").replace("    move sr,d0\n", ""));
        run_to_the_halt(&mut interpreter);
        let mut interpreter = machine(&program("68000").replace("    move ccr,d1\n", ""));
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 0);
    }

    #[test]
    fn rtd_returns_and_takes_the_arguments_off_the_stack() {
        let mut interpreter = m68010(
            "    move.l a7,a6
    pea 1
    pea 2
    bsr callee
    simhalt
callee:
    move.l 4(a7),d0
    add.l 8(a7),d0
    rtd #8",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 3);
        assert_eq!(a(&interpreter, 7), a(&interpreter, 6));
        assert!(
            interpreter.get_pretty_call_stack().is_empty(),
            "the call is off"
        );
        undo_restores_every_step(m68010(
            "    pea 1\n    bsr callee\n    simhalt\ncallee:\n    rtd #-4",
        ));
    }

    /// `bkpt` takes the illegal instruction exception, stacking its own
    /// address, as the 68010 does whatever answers its acknowledge cycle.
    #[test]
    fn bkpt_is_an_illegal_instruction() {
        let mut interpreter = m68010(
            "    move.l #illegal,$10
here:
    bkpt #5
    simhalt
illegal:
    move.l 2(a7),d0
    addq.l #2,2(a7)
    rte",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), label(&interpreter, "here"));
        let mut interpreter = m68010("    bkpt #0\n    simhalt");
        assert_eq!(interpreter.run(), Err(RuntimeError::IllegalInstruction));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: RuntimeError::IllegalInstruction,
                vector: Some(4)
            })
        );
    }

    /// `moves` reaches the one address space s68k has, whatever SFC and DFC
    /// hold: a byte into an address register is sign extended, and an
    /// address register written through its own `(An)+` is written as the
    /// step leaves it.
    #[test]
    fn moves_moves_between_a_register_and_memory() {
        let mut interpreter = m68010(
            "    moveq #1,d0
    movec d0,sfc
    movec d0,dfc
    lea data,a0
    move.l #$12345680,d1
    moves.w d1,(a0)
    moves.b (a0),a1
    moves.l (a0),d2
    lea data,a2
    moves.l a2,(a2)+
    simhalt
data:
    dc.l 0,0",
        );
        run_to_the_halt(&mut interpreter);
        let data = label(&interpreter, "data");
        assert_eq!(a(&interpreter, 1), 0x56, "the byte at data, sign extended");
        assert_eq!(d(&interpreter, 2), 0x5680_0000);
        assert_eq!(long(&interpreter, data), data + 4);
    }

    #[test]
    fn rtd_and_bkpt_assemble_only_for_the_68010_on() {
        for line in [
            "    rtd #4",
            "    bkpt #1",
            "    movec vbr,d0",
            "    moves.l (a0),d0",
        ] {
            let assembly = crate::assembler::assemble_source(&format!("    org $1000\n{line}\n"));
            assert!(
                assembly
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code() == "needs_a_later_processor"),
                "{line}"
            );
        }
        let assembly = crate::assembler::assemble_source("    opt p=68010\n    bkpt #8\n");
        assert!(assembly
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code() == "value_out_of_range"));
        let assembly = crate::assembler::assemble_source("vbr:\n    nop\n");
        assert!(
            assembly
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code() == "reserved_name_as_symbol"),
            "`vbr` is a register's name in every Program"
        );
        let _ = Access::Read;
    }
}

mod cycles {
    use super::*;

    /// The cycles of each step of `interpreter` until it pauses.
    fn step_cycles(interpreter: &mut Interpreter) -> Vec<u64> {
        let mut cycles = Vec::new();
        loop {
            let before = interpreter.get_cycles();
            let status = interpreter
                .step()
                .unwrap_or_else(|error| panic!("{error:?} after {cycles:?}"));
            cycles.push(interpreter.get_cycles() - before);
            if status != InterpreterStatus::Running {
                return cycles;
            }
        }
    }

    /// The 68010's own times, from section 9 of the 68000 User's Manual and,
    /// where it gives a maximum or differs, Moira's, which cputester checks.
    #[test]
    fn the_68010_takes_its_own_time() {
        let mut interpreter = m68010(
            "    move.l #handler,$80
    move sr,d0
    movec vbr,d1
    movec d1,vbr
    clr.l d2
    divu #7,d0
    mulu #$FFFF,d0
    trap #0
    simhalt
handler:
    rte",
        );
        let cycles = step_cycles(&mut interpreter);
        assert_eq!(
            cycles,
            [24, 4, 10, 12, 4, 78 + 2 * 15 + 4, 40 + 4, 38, 24, 0],
            "move.l, move sr, movec both ways, clr.l, divu, mulu, trap, rte, simhalt"
        );
        //the same on a 68000
        let mut interpreter =
            machine("    org $1000\n    move sr,d0\n    clr.l d2\n    mulu #$FFFF,d0\n    simhalt");
        assert_eq!(step_cycles(&mut interpreter), [6, 6, 38 + 32 + 4, 0]);
    }

    /// `move.w (a0)+,(a1)+` and `dbra` back to it: the `dbra` that branches
    /// back enters loop mode, and the next turns fetch nothing, 8 and 6 where
    /// they were 12 and 10; the one whose counter runs out ends it in 10, as
    /// Moira has it. The 68000 has no loop mode, and the 68010 none while T
    /// is set.
    #[test]
    fn a_one_instruction_loop_runs_in_loop_mode() {
        let body = "    lea data,a0
    lea data+16,a1
    moveq #3,d0
copy:
    move.w (a0)+,(a1)+
    dbra d0,copy
    simhalt
data:
    dcb.w 16,$1234";
        let mut interpreter = m68010(body);
        let cycles = step_cycles(&mut interpreter);
        assert_eq!(cycles, [12, 12, 4, 12, 10, 8, 6, 8, 6, 8, 10, 0]);
        let mut interpreter = machine(&format!("    org $1000\nstart:\n{body}"));
        let cycles = step_cycles(&mut interpreter);
        assert_eq!(cycles, [12, 12, 4, 12, 10, 12, 10, 12, 10, 12, 14, 0]);
        //undone and run again, a loop takes what it took
        let mut interpreter = m68010(body);
        let first = step_cycles(&mut interpreter);
        for _ in 0..5 {
            interpreter.undo().unwrap();
        }
        let mut again = first[..first.len() - 5].to_vec();
        again.extend(step_cycles(&mut interpreter));
        assert_eq!(again, first);
        undo_restores_every_step(m68010(body));
    }

    /// The `dbra` whose condition is true ends the loop in 12, and an IRQ
    /// ends it too: the loop enters it again after the handler's `rte`.
    #[test]
    fn a_loop_ends_on_its_condition_and_on_an_irq() {
        let mut interpreter = m68010(
            "    lea data,a0
    moveq #9,d0
search:
    tst.w (a0)+
    dbne d0,search
    simhalt
data:
    dc.w 0,0,0,5",
        );
        let cycles = step_cycles(&mut interpreter);
        //tst.w (a0)+ is 8 out of loop mode and 6 in it
        assert_eq!(cycles, [12, 4, 8, 10, 6, 6, 6, 6, 6, 12, 0]);

        let mut interpreter = m68010(
            "    move.l #irq,$60+4*2
    lea data,a0
    move.w #7,d0
spin:
    tst.w (a0)
    dbra d0,spin
    simhalt
irq:
    rte
data:
    dc.w 0",
        );
        for _ in 0..7 {
            interpreter.step().unwrap();
        }
        assert_eq!(interpreter.get_last_step_id(), 7);
        interpreter.raise_irq(2).unwrap();
        let cycles = step_cycles(&mut interpreter);
        //the IRQ, its `rte`, the `tst` out of loop mode, the `dbra` that enters it again
        assert_eq!(cycles[..4], [46, 24, 8, 10]);
    }
}
