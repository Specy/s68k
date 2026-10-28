//! The Memory map at run time (the asm-editor's `docs/design/m68k-machine.md`,
//! decision 15, and the plan's milestone C7): ROM ignores a write, read only
//! refuses one, protected refuses any access from user mode, invalid refuses
//! any access, each refusal the 68000's bus error, vector 2, with the group 0
//! frame. The checks are EASy68K's (`memoryMapCheck`, `mem_put` and `mem_req`
//! in its `UTILS.CPP`), but for the mode, which is the processor's own; the
//! debugger is not the program, and passes the map.

use crate::assembler::program::{MemoryRangeKind, Program};
use crate::instructions::{InterruptResult, RegisterOperand, Size};
use crate::interpreter::{
    Access, Interpreter, InterpreterOptions, InterpreterStatus, RuntimeError, Termination,
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
            history_size: 10_000,
        }),
    )
}

fn d(interpreter: &Interpreter, register: u8) -> u32 {
    interpreter.get_register_value(RegisterOperand::Data(register), Size::Long)
}

fn label(interpreter: &Interpreter, name: &str) -> u32 {
    interpreter
        .get_program()
        .symbols()
        .get(name)
        .unwrap_or_else(|| panic!("no label `{name}`"))
        .value as u32
}

fn bytes(interpreter: &Interpreter, address: usize, length: usize) -> Vec<u8> {
    interpreter
        .get_memory()
        .read_bytes(address, length)
        .unwrap()
        .to_vec()
}

fn bus_error(address: usize, size: Size, access: Access, range: MemoryRangeKind) -> RuntimeError {
    RuntimeError::BusError {
        address,
        size,
        access,
        range,
    }
}

/// Runs `program`, its first line `memory …`, then the access `line` in
/// supervisor mode, or in user mode when `user`, and answers how the run
/// ended.
fn access(range: &str, user: bool, line: &str) -> RuntimeResult {
    let mode = if user { "    andi #$dfff,sr\n" } else { "" };
    let mut interpreter = machine(&format!(
        "    memory {range}
    org $1000
start:
{mode}{line}
    simhalt
"
    ));
    let run = interpreter.run();
    (run, interpreter.get_termination().cloned())
}

type RuntimeResult = (
    Result<InterpreterStatus, RuntimeError>,
    Option<Termination>,
);

/// What a run that ends with `error`, and no handler at vector 2, answers.
fn ended_with(error: RuntimeError) -> RuntimeResult {
    (
        Err(error.clone()),
        Some(Termination::Exception {
            cause: error,
            vector: Some(2),
        }),
    )
}

fn paused() -> RuntimeResult {
    (Ok(InterpreterStatus::Paused), None)
}

mod each_kind {
    use super::*;
    use MemoryRangeKind::*;

    #[test]
    fn invalid_refuses_any_access_in_either_mode() {
        for user in [false, true] {
            assert_eq!(
                access("invalid $8000,$8fff", user, "    move.w $8000,d0"),
                ended_with(bus_error(0x8000, Size::Word, Access::Read, Invalid)),
                "user mode: {user}"
            );
            assert_eq!(
                access("invalid $8000,$8fff", user, "    move.b d0,$8fff"),
                ended_with(bus_error(0x8fff, Size::Byte, Access::Write, Invalid)),
            );
            assert_eq!(
                access("invalid $8000,$8fff", user, "    move.w $9000,d0"),
                paused(),
                "past the range"
            );
        }
    }

    /// A write counts every byte it stores, and a read only its first, as in
    /// EASy68K.
    #[test]
    fn a_write_is_refused_by_any_byte_and_a_read_by_its_first() {
        assert_eq!(
            access("invalid $8000,$8fff", false, "    move.l d0,$7ffe"),
            ended_with(bus_error(0x7ffe, Size::Long, Access::Write, Invalid)),
        );
        assert_eq!(
            access("invalid $8000,$8fff", false, "    move.l $7ffe,d0"),
            paused()
        );
        // The address is read in the 24 bits of the bus, and the error names
        // all 32 the program used.
        assert_eq!(
            access("invalid $8000,$8fff", false, "    move.w $ff008000,d0"),
            ended_with(bus_error(0xff00_8000, Size::Word, Access::Read, Invalid)),
        );
    }

    #[test]
    fn protected_refuses_any_access_from_user_mode() {
        assert_eq!(
            access(
                "protected $8000,$8fff",
                false,
                "    move.w #1,$8000\n    move.w $8000,d0"
            ),
            paused(),
            "supervisor mode reads and writes it"
        );
        assert_eq!(
            access("protected $8000,$8fff", true, "    move.w $8000,d0"),
            ended_with(bus_error(0x8000, Size::Word, Access::Read, Protected)),
        );
        assert_eq!(
            access("protected $8000,$8fff", true, "    move.w d0,$8000"),
            ended_with(bus_error(0x8000, Size::Word, Access::Write, Protected)),
        );
    }

    #[test]
    fn read_only_refuses_a_write_in_either_mode() {
        for user in [false, true] {
            assert_eq!(
                access("read $8000,$8fff", user, "    move.w $8000,d0"),
                paused(),
                "a read is fine"
            );
            assert_eq!(
                access("read $8000,$8fff", user, "    addq.w #1,$8000"),
                ended_with(bus_error(0x8000, Size::Word, Access::Write, Read)),
                "a read-modify-write reads, then is refused its write"
            );
        }
    }

    #[test]
    fn rom_ignores_a_write_in_either_mode() {
        for user in [false, true] {
            let mode = if user { "    andi #$dfff,sr\n" } else { "" };
            let mut interpreter = machine(&format!(
                "    memory rom $8000,$8fff
    org $1000
start:
{mode}    move.w #$1234,$8000
    move.w $8000,d0
    simhalt
"
            ));
            assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
            assert_eq!(d(&interpreter, 0) as u16, 0xffff, "memory kept what it held");
            assert_eq!(bytes(&interpreter, 0x8000, 2), [0xff, 0xff]);
        }
    }

    /// A ROM over the program's own code keeps the code: the store into it
    /// is no store, changes no instruction, is no Code write, and journals
    /// nothing.
    #[test]
    fn a_rom_over_the_programs_code_keeps_it() {
        let mut interpreter = machine(
            "    memory rom $1000,$10ff
    org $1000
start:
    move.w #$4e71,patch
patch:
    moveq #5,d0
    simhalt
",
        );
        interpreter.step().unwrap();
        assert!(interpreter.get_code_write().is_none());
        assert!(interpreter.get_last_steps(1)[0].get_mutations().is_empty());
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
        assert_eq!(d(&interpreter, 0), 5, "the moveq ran as it was assembled");
        let patch = label(&interpreter, "patch") as usize;
        assert_eq!(bytes(&interpreter, patch, 2), [0x70, 0x05]);
    }
}

mod bus_errors {
    use super::*;

    /// A handler that records the group 0 frame: the status word in D1, the
    /// access address in D2, the instruction register in D3, the status
    /// register in D4 and the program counter in D5. It drops the frame's
    /// first 8 bytes and returns.
    const HANDLER: &str = "
handler:
    move.w (a7),d1
    move.l 2(a7),d2
    move.w 6(a7),d3
    move.w 8(a7),d4
    move.l 10(a7),d5
    addq.l #8,a7
    rte
";

    fn with_handler(lines: &str) -> Interpreter {
        let mut interpreter = machine(&format!(
            "    memory invalid $8000,$8fff
    org $1000
start:
    move.l #handler,$8
{lines}
after:
    simhalt
{HANDLER}"
        ));
        assert_eq!(
            interpreter.run(),
            Ok(InterpreterStatus::Paused),
            "{:?}",
            interpreter.get_termination()
        );
        interpreter
    }

    /// The frame is the 68000's: R/W says read, I/N is clear, the function
    /// code is the mode's data space, and the program counter is the one an
    /// address error at the same access stacks.
    #[test]
    fn a_read_pushes_the_long_frame_of_a_read() {
        let interpreter = with_handler("fault:\n    move.w $8000,d0");
        let opcode = 0x3039;
        assert_eq!(d(&interpreter, 1) as u16, (opcode & 0xffe0) | 0x15);
        assert_eq!(d(&interpreter, 2), 0x8000);
        assert_eq!(d(&interpreter, 3) as u16, opcode);
        assert_eq!(d(&interpreter, 4) as u16, 0x2000);
        assert_eq!(d(&interpreter, 5), label(&interpreter, "after"));
        assert_eq!(interpreter.get_ssp(), 0x0100_0000);
    }

    #[test]
    fn a_write_from_user_mode_says_write_and_user_data() {
        let interpreter = with_handler("    andi #$dfff,sr\n    move.w d0,$8000");
        assert_eq!(d(&interpreter, 1) & 0x1f, 0x01, "write, I/N clear, user data");
        assert_eq!(d(&interpreter, 4) & 0x2000, 0, "the SR of user mode");
    }

    /// An instruction fetched from an invalid range is a bus error in
    /// program space, at the first word that is refused, and stacks the
    /// address of the instruction that could not be fetched.
    #[test]
    fn a_fetch_from_an_invalid_range_is_one_in_program_space() {
        let mut interpreter = machine(
            "    memory invalid $8000,$8fff
    org $1000
start:
    jmp $8000
",
        );
        let error = bus_error(0x8000, Size::Word, Access::Program, MemoryRangeKind::Invalid);
        assert_eq!(interpreter.run(), Err(error.clone()));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: error,
                vector: Some(2),
            })
        );
        // With a handler: a supervisor program read, at $8000.
        let mut interpreter = machine(&format!(
            "    memory invalid $8000,$8fff
    org $1000
start:
    move.l #handler,$8
    jmp $8000
{HANDLER}"
        ));
        for _ in 0..3 {
            interpreter.step().unwrap();
        }
        assert_eq!(interpreter.get_pc(), label(&interpreter, "handler") as usize);
        let frame = bytes(&interpreter, interpreter.get_ssp() as usize, 14);
        assert_eq!(&frame[0..2], [0xff, 0xf6], "$FFFF's high bits, read, supervisor program");
        assert_eq!(&frame[2..6], [0x00, 0x00, 0x80, 0x00]);
        assert_eq!(&frame[10..14], [0x00, 0x00, 0x80, 0x00], "the stacked PC");
    }

    /// Each word of an instruction is fetched as a read, so one whose
    /// extension word is in the range is refused there, though it starts
    /// before it, in a page the range does not reach.
    #[test]
    fn an_extension_word_in_the_range_is_refused() {
        let mut interpreter = machine(
            "    memory invalid $8000,$8fff
    org $7ffc
start:
    move.l #$12345,d0
",
        );
        assert_eq!(
            interpreter.run(),
            Err(bus_error(
                0x8000,
                Size::Word,
                Access::Program,
                MemoryRangeKind::Invalid
            ))
        );
        assert_eq!(d(&interpreter, 0), 0, "it did not run");
    }

    /// Only the instructions a word of which an invalid or protected range
    /// reaches are left out of the decode cache: the code beside one is kept,
    /// and a write into it is a Code write as before.
    #[test]
    fn code_beside_a_checked_range_is_kept() {
        let mut interpreter = machine(
            "    memory protected $1010,$1fff
    org $1000
start:
    move.w #$7005,patch
patch:
    moveq #1,d0
    simhalt
",
        );
        assert!(interpreter.get_memory().cache().get(0x1000).is_some());
        interpreter.step().unwrap();
        assert!(interpreter.get_code_write().is_some());
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
        assert_eq!(d(&interpreter, 0), 5, "the new bytes ran");
        let mut inside = machine(
            "    memory protected $1000,$1fff
    org $1000
start:
    moveq #1,d0
    simhalt
",
        );
        assert!(inside.get_memory().cache().get(0x1000).is_none());
        assert_eq!(inside.run(), Ok(InterpreterStatus::Paused), "supervisor mode runs it");
    }

    /// Code in a protected range runs in supervisor mode and is refused to
    /// user mode, each fetch checked in the mode the processor is in.
    #[test]
    fn protected_code_runs_in_supervisor_mode_only() {
        let mut interpreter = machine(
            "    memory protected $2000,$2fff
    org $1000
start:
    jsr routine
    andi #$dfff,sr
    jsr routine
    simhalt
    org $2000
routine:
    addq.l #1,d0
    rts
",
        );
        assert_eq!(
            interpreter.run(),
            Err(bus_error(
                0x2000,
                Size::Word,
                Access::Program,
                MemoryRangeKind::Protected
            ))
        );
        assert_eq!(d(&interpreter, 0), 1, "the first call ran");
    }

    /// Reading the vector of the bus error out of an invalid range is a
    /// double fault, and so is pushing a frame into one: the processor halts.
    #[test]
    fn a_bus_error_while_taking_an_exception_halts() {
        let mut interpreter = machine(
            "    memory invalid $0,$ff
    org $1000
start:
    move.w $50,d0
",
        );
        let fault = bus_error(0x8, Size::Long, Access::Read, MemoryRangeKind::Invalid);
        assert_eq!(interpreter.run(), Err(fault.clone()));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Halted {
                cause: fault,
                vector: 2,
            })
        );
        let mut interpreter = machine(
            "    memory invalid $f00000,$ffffff
    org $1000
start:
    move.l #handler,$80
    trap #0
    simhalt
handler:
    rte
",
        );
        // The program counter's low word goes first, just below the top.
        let fault = bus_error(0xff_fffe, Size::Word, Access::Write, MemoryRangeKind::Invalid);
        assert_eq!(interpreter.run(), Err(fault.clone()));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Halted {
                cause: fault,
                vector: 32,
            })
        );
    }

    /// A handler that cannot be fetched: a group 1 or 2 exception's raises
    /// the bus error of its first fetch, a group 0 one, and a group 0
    /// exception's halts the processor, a double bus fault, as a 68000 halts
    /// and EASy68K does after two bus or address errors in a row. It used to
    /// take the bus error again, through the same handler, for ever.
    #[test]
    fn a_bus_error_whose_handler_cannot_be_fetched_halts() {
        let mut interpreter = machine(
            "    memory invalid $8000,$8fff
    org $1000
start:
    move.l #$8000,$8
    move.l #$8000,$14
    moveq #0,d1
    divu d1,d0
    simhalt
",
        );
        let fault = bus_error(0x8000, Size::Word, Access::Program, MemoryRangeKind::Invalid);
        assert_eq!(interpreter.run_with_limit(200), Err(fault.clone()));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Halted {
                cause: fault,
                vector: 2,
            })
        );
        //the division's frame and the bus error's, and no other
        assert_eq!(interpreter.get_ssp(), 0x0100_0000 - 6 - 14);
        assert_eq!(interpreter.get_pc(), 0x8000);
        //the same for an address error whose handler is out of reach
        let mut interpreter = machine(
            "    memory invalid $8000,$8fff
    org $1000
start:
    move.l #$8000,$c
    move.w $1001,d0
    simhalt
",
        );
        let fault = bus_error(0x8000, Size::Word, Access::Program, MemoryRangeKind::Invalid);
        assert_eq!(interpreter.run_with_limit(200), Err(fault.clone()));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Halted {
                cause: fault,
                vector: 3,
            })
        );
        interpreter.undo().unwrap();
        assert!(!interpreter.has_terminated(), "undo takes the halt back");
        assert_eq!(interpreter.get_ssp(), 0x0100_0000);
    }

    /// The vector is read in supervisor mode, so a protected vector table
    /// serves an exception taken in user mode.
    #[test]
    fn a_protected_vector_table_serves_user_mode() {
        let mut interpreter = machine(
            "    memory protected $0,$3ff
    org $1000
start:
    move.l #handler,$80
    andi #$dfff,sr
    trap #0
    simhalt
handler:
    moveq #9,d7
    rte
",
        );
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
        assert_eq!(d(&interpreter, 7), 9);
    }

    /// Undo takes a bus error back, its frame and the handler's run with it.
    #[test]
    fn undo_takes_back_a_bus_error() {
        let mut interpreter = machine(&format!(
            "    memory read $8000,$8fff
    org $1000
start:
    move.l #handler,$8
    move.w d0,$8000
    simhalt
{HANDLER}"
        ));
        interpreter.step().unwrap();
        let before = (
            interpreter.get_cpu().get_register_values(),
            interpreter.get_sr(),
            interpreter.get_pc(),
            bytes(&interpreter, 0xffff00, 0x100),
        );
        interpreter.step().unwrap();
        assert_eq!(interpreter.get_pc(), label(&interpreter, "handler") as usize);
        assert_eq!(interpreter.get_pretty_call_stack().len(), 1);
        interpreter.undo().unwrap();
        let after = (
            interpreter.get_cpu().get_register_values(),
            interpreter.get_sr(),
            interpreter.get_pc(),
            bytes(&interpreter, 0xffff00, 0x100),
        );
        assert_eq!(after, before);
        assert!(interpreter.get_pretty_call_stack().is_empty());
        assert_eq!(*interpreter.get_status(), InterpreterStatus::Running);
    }
}

mod trap_buffers {
    use super::*;
    use MemoryRangeKind::*;

    fn trap(range: &str, setup: &str) -> Interpreter {
        machine(&format!(
            "    memory {range}
    org $1000
start:
{setup}
    trap #15
    moveq #1,d7
    simhalt
    org $8000
text:
    dc.b 'hi',0
"
        ))
    }

    /// A string a task displays is checked against invalid, as EASy68K
    /// checks it, from its start.
    #[test]
    fn a_string_in_an_invalid_range_is_a_bus_error() {
        let mut interpreter = trap("invalid $8000,$8fff", "    lea text,a1\n    moveq #13,d0");
        let error = bus_error(0x8000, Size::Byte, Access::Read, Invalid);
        assert_eq!(interpreter.run(), Err(error.clone()));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: error,
                vector: Some(2),
            })
        );
        // One that ends before the range is not refused, though its NUL is
        // the byte before it.
        let mut interpreter = trap("invalid $8003,$8fff", "    lea text,a1\n    moveq #13,d0");
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Interrupt));
    }

    /// EASy68K checks a task's string against invalid alone: a protected one
    /// is displayed in user mode, and a read only one is read.
    #[test]
    fn a_string_in_a_protected_range_is_read_from_user_mode() {
        let mut interpreter = trap(
            "protected $8000,$8fff",
            "    lea text,a1\n    moveq #13,d0\n    andi #$dfff,sr",
        );
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Interrupt));
    }

    /// Task 53 checks the bytes it reads a file into against invalid, read
    /// only and ROM, before it looks at the file: read only is a bus error,
    /// a write; ROM takes nothing, and the task does nothing, D0 untouched.
    #[test]
    fn task_53_checks_where_it_reads_a_file_into() {
        let setup = "    lea text,a1\n    moveq #0,d1\n    moveq #2,d2\n    moveq #53,d0";
        let mut interpreter = trap("read $8000,$8fff", setup);
        assert_eq!(
            interpreter.run(),
            Err(bus_error(0x8000, Size::Byte, Access::Write, Read))
        );
        let mut interpreter = trap("rom $8000,$8fff", setup);
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
        assert_eq!(d(&interpreter, 0), 53, "D0 is as the task found it");
        assert_eq!(d(&interpreter, 7), 1, "and the program went on");
        // Outside the map the file is read as before.
        let mut interpreter = trap("rom $9000,$9fff", setup);
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Interrupt));
    }

    /// A buffer of no bytes is checked as EASy68K checks it,
    /// `loc + bytes - 1 >= start && loc <= end`: one that starts past the
    /// first byte of a range and inside it is refused, and one on the first
    /// byte is not, as `loc - 1` is below the start.
    #[test]
    fn a_buffer_of_no_bytes_is_refused_strictly_inside_a_range() {
        //task 13 with A1 on the NUL of `hi`
        let on_the_nul = "    lea text+2,a1\n    moveq #13,d0";
        let mut interpreter = trap("invalid $8000,$8fff", on_the_nul);
        assert_eq!(
            interpreter.run(),
            Err(bus_error(0x8002, Size::Byte, Access::Read, Invalid))
        );
        let mut interpreter = trap("invalid $8002,$8fff", on_the_nul);
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Interrupt));
        //task 53 reading no bytes
        let none = "    lea text+2,a1\n    moveq #0,d1\n    moveq #0,d2\n    moveq #53,d0";
        let mut interpreter = trap("read $8000,$8fff", none);
        assert_eq!(
            interpreter.run(),
            Err(bus_error(0x8002, Size::Byte, Access::Write, Read))
        );
        let mut interpreter = trap("rom $8000,$8fff", none);
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
        assert_eq!(d(&interpreter, 0), 53, "ROM takes nothing, and D0 is as it was");
        let mut interpreter = trap("read $8002,$8fff", none);
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Paused));
        assert_eq!(d(&interpreter, 0) & 0xFFFF, 2, "a read of no bytes, as before");
    }

    /// Task 54 checks the bytes it writes a file from against invalid.
    #[test]
    fn task_54_checks_where_it_writes_a_file_from() {
        let setup = "    lea text,a1\n    moveq #0,d1\n    moveq #2,d2\n    moveq #54,d0";
        let mut interpreter = trap("invalid $8001,$8fff", setup);
        assert_eq!(
            interpreter.run(),
            Err(bus_error(0x8000, Size::Byte, Access::Read, Invalid))
        );
        let mut interpreter = trap("read $8000,$8fff", setup);
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Interrupt));
    }

    /// EASy68K's task 2 writes the line it reads with no check, so it reaches
    /// a ROM range too.
    #[test]
    fn task_2_writes_its_line_past_the_map() {
        let mut interpreter = trap("rom $8000,$8fff", "    lea text,a1\n    moveq #2,d0");
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Interrupt));
        interpreter
            .answer_interrupt(InterruptResult::ReadKeyboardString("ok".to_string()))
            .unwrap();
        assert_eq!(bytes(&interpreter, 0x8000, 3), [b'o', b'k', 0]);
    }
}

mod the_debugger_passes_the_map {
    use super::*;

    /// A Poke, a Testcase's starting memory and the host's reads go past the
    /// map, and undo puts back what a Poke wrote there.
    #[test]
    fn pokes_and_host_reads_go_past_the_map() {
        let mut interpreter = machine(
            "    memory invalid $8000,$8fff
    memory rom $9000,$9fff
    org $1000
start:
    simhalt
",
        );
        assert_eq!(bytes(&interpreter, 0x8000, 2), [0xff, 0xff]);
        interpreter.write_memory_bytes(0x8000, &[1, 2]).unwrap();
        assert_eq!(bytes(&interpreter, 0x8000, 2), [1, 2], "a Testcase's memory");
        interpreter.begin_poke().unwrap();
        interpreter.write_memory_bytes(0x9000, &[3, 4]).unwrap();
        interpreter.end_poke().unwrap();
        assert_eq!(bytes(&interpreter, 0x9000, 2), [3, 4], "a Poke into ROM");
        assert_eq!(interpreter.disassemble_at(0x8000).bytes, [1, 2]);
        interpreter.undo().unwrap();
        assert_eq!(bytes(&interpreter, 0x9000, 2), [0xff, 0xff]);
    }

    /// The watched range sees no store a ROM range ignored, as there is none,
    /// and sees the others as before.
    #[test]
    fn a_watched_range_sees_no_store_a_rom_ignored() {
        let mut interpreter = machine(
            "    memory rom $8000,$80ff
    org $1000
start:
    move.w #1,$8000
    move.w #2,$8100
    simhalt
",
        );
        interpreter.watch_memory(0x8000, 0x8200).unwrap();
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::Running));
        assert!(interpreter.take_watched_writes().is_empty());
        assert_eq!(interpreter.step(), Ok(InterpreterStatus::WatchedWrite));
        assert_eq!(interpreter.take_watched_writes().len(), 1);
    }
}
