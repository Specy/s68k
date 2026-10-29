//! Exceptions, privilege and trace (ADR 0008; the asm-editor's
//! `docs/design/m68k-machine.md`, decisions 9 to 12).
//!
//! Each program installs its handlers by writing the vectors, as a 68000
//! program does, and the tests read what the handler saw and what is left
//! after its `rte`. A vector the program does not write holds `$FFFFFFFF`,
//! which is no handler: the program ends with the error, as it did before
//! exceptions were taken. Undo is checked across all of it against a snapshot
//! of every register, both stack pointers, the status register, the call
//! stack and the memory the programs touch.

use crate::assembler::program::Program;
use crate::debugger::{FrameKind, MutationOperation};
use crate::instructions::{InterruptResult, RegisterOperand, Size};
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

fn long(interpreter: &Interpreter, address: usize) -> u32 {
    interpreter.get_memory().read_long(address).unwrap()
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
    usp: u32,
    ssp: u32,
    sr: u16,
    pc: usize,
    status: InterpreterStatus,
    termination: Option<Termination>,
    call_stack: Vec<(usize, usize, FrameKind, Option<u8>)>,
    memory: Vec<u8>,
}

fn snapshot(interpreter: &Interpreter) -> Snapshot {
    let memory = interpreter.get_memory();
    let mut bytes = Vec::new();
    for (start, end) in [
        (0x0, 0x400),
        (0x1000, 0x1400),
        (0x7F00, 0x9100),
        (0xFFFF00, 0x1000000),
    ] {
        bytes.extend_from_slice(&memory.read_bytes(start, end - start).unwrap());
    }
    Snapshot {
        registers: interpreter.get_cpu().get_register_values(),
        usp: interpreter.get_usp(),
        ssp: interpreter.get_ssp(),
        sr: interpreter.get_sr(),
        pc: interpreter.get_pc(),
        status: *interpreter.get_status(),
        termination: interpreter.get_termination().cloned(),
        call_stack: interpreter
            .get_pretty_call_stack()
            .iter()
            .map(|frame| {
                (
                    frame.address,
                    frame.source_address,
                    frame.kind,
                    frame.vector,
                )
            })
            .collect(),
        memory: bytes,
    }
}

/// Steps the program to its end, a snapshot before every step, then undoes
/// every step and checks that each undo puts back the snapshot taken before
/// the step it undid. Answers how many steps it took.
fn undo_restores_every_step(code: &str) -> usize {
    let mut interpreter = machine(code);
    let mut snapshots = vec![snapshot(&interpreter)];
    loop {
        let stepped = interpreter.step();
        snapshots.push(snapshot(&interpreter));
        match stepped {
            Ok(InterpreterStatus::Running) => continue,
            _ => break,
        }
    }
    let steps = snapshots.len() - 1;
    snapshots.pop();
    while let Some(before) = snapshots.pop() {
        interpreter.undo().expect("a step to undo");
        assert_eq!(
            snapshot(&interpreter),
            before,
            "undoing step {}",
            snapshots.len()
        );
    }
    assert!(!interpreter.can_undo());
    steps
}

/// A handler that records the frame of groups 1 and 2: the stacked PC in D6,
/// the stacked SR in D7, A7 in D5, and the vector count in D4.
const RECORDING_HANDLER: &str = "
handler:
    move.l 2(a7),d6
    move.w (a7),d7
    move.l a7,d5
    addq.l #1,d4
    rte
";

mod with_and_without_a_handler {
    use super::*;

    #[test]
    fn a_division_by_zero_runs_the_handler_and_rte_returns_past_the_divide() {
        let mut interpreter = machine(&format!(
            "    org $1000
start:
    move.l #handler,$14
    moveq #0,d1
    move.l #10,d0
divide:
    divu d1,d0
after:
    move.l #1,d2
    simhalt
{RECORDING_HANDLER}"
        ));
        run_to_the_halt(&mut interpreter);
        assert_eq!(
            d(&interpreter, 6),
            label(&interpreter, "after"),
            "the next instruction is stacked"
        );
        assert_eq!(
            d(&interpreter, 7) as u16,
            0x2000,
            "the status register from before"
        );
        assert_eq!(
            d(&interpreter, 5),
            0x00FF_FFFA,
            "a frame of six bytes on the SSP"
        );
        assert_eq!(d(&interpreter, 4), 1);
        assert_eq!(
            d(&interpreter, 2),
            1,
            "the program went on after the divide"
        );
        assert_eq!(interpreter.get_ssp(), 0x0100_0000, "rte took the frame off");
        assert_eq!(interpreter.get_sr() & 0xFF00, 0x2000);
        assert!(interpreter.get_pretty_call_stack().is_empty());
    }

    #[test]
    fn a_division_by_zero_with_no_handler_ends_the_program_as_before() {
        let mut interpreter = machine(
            "    org $1000
start:
    moveq #0,d1
    move.l #10,d0
    divu d1,d0
    move.l #1,d2
    simhalt
",
        );
        assert_eq!(interpreter.run(), Err(RuntimeError::DivisionByZero));
        assert_eq!(
            *interpreter.get_status(),
            InterpreterStatus::TerminatedWithException
        );
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: RuntimeError::DivisionByZero,
                vector: Some(5),
            })
        );
        assert_eq!(interpreter.get_ssp(), 0x0100_0000, "no frame was pushed");
        assert_eq!(interpreter.get_sr(), 0x2000);
        assert_eq!(d(&interpreter, 2), 0);
    }

    #[test]
    fn a_vector_holding_an_odd_address_holds_no_handler() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #$2001,$80
    trap #0
    simhalt
",
        );
        assert_eq!(
            interpreter.run(),
            Err(RuntimeError::UnhandledTrap { vector: 32 })
        );
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: RuntimeError::UnhandledTrap { vector: 32 },
                vector: Some(32),
            })
        );
    }

    /// The program counter each exception stacks: the instruction's own
    /// address for groups 1, the next instruction's for group 2. A handler of
    /// group 1 steps past the instruction itself, as a real one decides to.
    #[test]
    fn every_exception_stacks_the_program_counter_of_its_group() {
        let cases = [
            ("illegal", 4, "", "    illegal", true, 2),
            ("chk", 6, "    move.w #5,d0", "    chk #3,d0", false, 0),
            ("trapv", 7, "    move.w #2,ccr", "    trapv", false, 0),
            ("line A", 10, "", "    dc.w $A123", true, 2),
            ("line F", 11, "", "    dc.w $F000", true, 2),
            ("trap #0", 32, "", "    trap #0", false, 0),
            ("trap #14", 46, "", "    trap #14", false, 0),
            (
                "an unknown task",
                47,
                "    move.b #99,d0",
                "    trap #15",
                false,
                0,
            ),
        ];
        for (name, vector, setup, raising, own_address, size) in cases {
            let skip = match own_address {
                true => format!("    addq.l #{size},2(a7)\n"),
                false => String::new(),
            };
            let mut interpreter = machine(&format!(
                "    org $1000
start:
    move.l #handler,${:x}
{setup}
raise:
{raising}
after:
    simhalt
handler:
    move.l 2(a7),d6
    addq.l #1,d4
{skip}    rte
",
                vector as u32 * 4
            ));
            run_to_the_halt(&mut interpreter);
            let expected = match own_address {
                true => label(&interpreter, "raise"),
                false => label(&interpreter, "after"),
            };
            assert_eq!(d(&interpreter, 6), expected, "{name} stacks ${expected:x}");
            assert_eq!(d(&interpreter, 4), 1, "{name} ran the handler once");
        }
    }

    #[test]
    fn every_exception_with_no_handler_ends_the_program_naming_its_vector() {
        let cases = [
            ("    illegal", RuntimeError::IllegalInstruction, 4),
            (
                "    move.w #5,d0\n    chk #3,d0",
                RuntimeError::ChkOutOfBounds { value: 5, bound: 3 },
                6,
            ),
            (
                "    move.w #2,ccr\n    trapv",
                RuntimeError::OverflowException,
                7,
            ),
            (
                "    andi.w #$dfff,sr\n    move.w #0,sr",
                RuntimeError::PrivilegeViolation,
                8,
            ),
            ("    ori.w #$8000,sr\n    nop", RuntimeError::Trace, 9),
            ("    dc.w $A123", RuntimeError::LineA(0xA123), 10),
            ("    dc.w $F001", RuntimeError::LineF(0xF001), 11),
            (
                "    trap #3",
                RuntimeError::UnhandledTrap { vector: 35 },
                35,
            ),
            (
                "    move.b #26,d0\n    trap #15",
                RuntimeError::UnsupportedTrapTask { task: 26 },
                47,
            ),
        ];
        for (code, error, vector) in cases {
            let mut interpreter = machine(&format!("    org $1000\nstart:\n{code}\n    simhalt\n"));
            assert_eq!(interpreter.run(), Err(error.clone()), "{code}");
            assert_eq!(
                interpreter.get_termination(),
                Some(&Termination::Exception {
                    cause: error,
                    vector: Some(vector),
                }),
                "{code}"
            );
        }
    }

    /// One of EASy68K's own tasks that s68k refuses is not vector 47's: it
    /// ends the program whatever vector 47 holds.
    #[test]
    fn a_refused_task_of_easy68ks_is_no_exception() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #handler,$bc
    move.b #10,d0
    trap #15
    simhalt
handler:
    rte
",
        );
        assert_eq!(
            interpreter.run(),
            Err(RuntimeError::UnsupportedTrapTask { task: 10 })
        );
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: RuntimeError::UnsupportedTrapTask { task: 10 },
                vector: None,
            })
        );
    }

    /// Task 32 with D1.B of 5 is EASy68K's "Enable exceptions", which s68k
    /// takes always: accepted, and nothing else changes.
    #[test]
    fn task_32_number_5_is_accepted_and_does_nothing() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.b #32,d0
    move.b #5,d1
    trap #15
    simhalt
",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 32);
        assert_eq!(d(&interpreter, 1), 5);
        let mut other =
            machine("    org $1000\nstart:\n    move.b #32,d0\n    move.b #0,d1\n    trap #15\n");
        assert_eq!(
            other.run(),
            Err(RuntimeError::UnsupportedTrapTask { task: 32 })
        );
    }

    /// The group 0 frame: the status word, the access address, the
    /// instruction register, the status register and the program counter,
    /// 14 bytes, which the handler takes off itself before `rte`.
    #[test]
    fn an_address_error_pushes_the_long_frame() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #handler,$c
fault:
    move.w $2001,d0
after:
    simhalt
handler:
    move.w (a7),d1
    move.l 2(a7),d2
    move.w 6(a7),d3
    move.w 8(a7),d4
    move.l 10(a7),d5
    move.l a7,d6
    addq.l #8,a7
    rte
",
        );
        run_to_the_halt(&mut interpreter);
        let opcode = interpreter
            .get_memory()
            .read_word(label(&interpreter, "fault") as usize)
            .unwrap();
        assert_eq!(
            d(&interpreter, 1) as u16,
            (opcode & 0xFFE0) | 0x15,
            "a supervisor data read"
        );
        assert_eq!(d(&interpreter, 2), 0x2001, "the access address");
        assert_eq!(
            d(&interpreter, 3) as u16,
            opcode,
            "the instruction register"
        );
        assert_eq!(d(&interpreter, 4) as u16, 0x2000);
        assert_eq!(
            d(&interpreter, 5),
            label(&interpreter, "after"),
            "the prefetch had got past it"
        );
        assert_eq!(d(&interpreter, 6), 0x0100_0000 - 14);
        assert_eq!(interpreter.get_ssp(), 0x0100_0000);
    }

    /// A jump to an odd address faults in the jump, with the access in
    /// program space, and the target as the access address.
    #[test]
    fn a_jump_to_an_odd_address_faults_in_the_jump() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #handler,$c
    jmp $3001
handler:
    move.w (a7),d1
    move.l 2(a7),d2
    simhalt
",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 1) & 0x1F, 0x16, "a supervisor program read");
        assert_eq!(d(&interpreter, 2), 0x3001);
    }

    #[test]
    fn an_address_error_with_no_handler_names_the_access() {
        let mut interpreter = machine("    org $1000\nstart:\n    move.l d0,$2003\n    simhalt\n");
        let error = RuntimeError::AddressError {
            address: 0x2003,
            size: Size::Long,
            access: Access::Write,
        };
        assert_eq!(interpreter.run(), Err(error.clone()));
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Exception {
                cause: error,
                vector: Some(3),
            })
        );
    }
}

mod supervisor_and_user_mode {
    use super::*;

    /// An exception from user mode switches to the supervisor stack, and
    /// `rte` switches back: the frame is on the SSP and the USP is untouched.
    #[test]
    fn an_exception_from_user_mode_runs_on_the_supervisor_stack() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #handler,$80
    move.l #$8000,a0
    move.l a0,usp
    andi.w #$dfff,sr
    trap #0
after:
    move.l a7,d1
    simhalt
handler:
    move.l a7,d2
    move.w (a7),d3
    move.l usp,a1
    rte
",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(
            d(&interpreter, 2),
            0x00FF_FFFA,
            "the handler ran on the SSP"
        );
        assert_eq!(d(&interpreter, 3) & 0x2000, 0, "from user mode");
        assert_eq!(a(&interpreter, 1), 0x8000, "the USP kept aside");
        assert_eq!(d(&interpreter, 1), 0x8000, "rte made A7 the USP again");
        assert_eq!(interpreter.get_sr() & 0x2000, 0, "in user mode");
        assert_eq!(interpreter.get_usp(), 0x8000);
        assert_eq!(interpreter.get_ssp(), 0x0100_0000);
    }

    /// The usual way into user mode: a frame built by hand, and `rte`.
    #[test]
    fn rte_into_user_mode_swaps_the_stacks() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #$9000,a0
    move.l a0,usp
    pea user
    move.w #$0000,-(a7)
    rte
user:
    move.l a7,d0
    simhalt
",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 0x9000);
        assert_eq!(interpreter.get_ssp(), 0x0100_0000, "the frame was popped");
        assert_eq!(interpreter.get_sr(), 0x0000);
        assert!(
            interpreter.get_pretty_call_stack().is_empty(),
            "rte popped no call"
        );
    }

    #[test]
    fn a_write_of_s_swaps_the_stacks_both_ways() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #$8000,a0
    move.l a0,usp
    move.w #$0000,sr
    move.l a7,d0
    simhalt
",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 0), 0x8000, "A7 is the USP in user mode");
        assert_eq!(interpreter.get_ssp(), 0x0100_0000);
    }

    /// Every privileged instruction raises a privilege violation in user
    /// mode, stacking its own address, so that the handler can skip it.
    #[test]
    fn every_privileged_instruction_is_a_privilege_violation_in_user_mode() {
        let cases = [
            ("    move.w #$2700,sr", 4),
            ("    andi.w #$ffff,sr", 4),
            ("    ori.w #$0700,sr", 4),
            ("    eori.w #$0700,sr", 4),
            ("    rte", 2),
            ("    move.l usp,a0", 2),
            ("    move.l a0,usp", 2),
            ("    dc.w $4e72,$2000", 4),
            ("    dc.w $4e70", 2),
        ];
        for (code, size) in cases {
            let mut interpreter = machine(&format!(
                "    org $1000
start:
    move.l #handler,$20
    move.l #$8000,a0
    move.l a0,usp
    andi.w #$dfff,sr
privileged:
{code}
after:
    move.w sr,d1
    simhalt
handler:
    move.l 2(a7),d6
    addq.l #{size},2(a7)
    rte
"
            ));
            run_to_the_halt(&mut interpreter);
            assert_eq!(
                d(&interpreter, 6),
                label(&interpreter, "privileged"),
                "{code}"
            );
            assert_eq!(d(&interpreter, 1) & 0x2000, 0, "{code}: still in user mode");
        }
    }

    /// Each stack pointer has a top of its own, which the memory view draws
    /// the Stack region up to: a handler entered from user mode is on the
    /// supervisor stack, whose top is its own, and the `rte` back finds the
    /// user stack's top where the program left it, with what it pushed under
    /// it. One top swapped from stack to stack drew an empty region in the
    /// handler and lost the user stack's data after it.
    #[test]
    fn each_stack_pointer_keeps_its_own_top() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #handler,$80
    lea $8000,a0
    move.l a0,usp
    andi.w #$dfff,sr
    move.l #1,-(a7)
    trap #0
back:
    simhalt
handler:
    nop
    rte
",
        );
        let mut seen = vec![(interpreter.get_pc(), interpreter.get_stack_top())];
        let mut step = |interpreter: &mut Interpreter| {
            interpreter.step().unwrap();
            seen.push((interpreter.get_pc(), interpreter.get_stack_top()));
        };
        for _ in 0..3 {
            step(&mut interpreter);
        }
        assert_eq!(interpreter.get_stack_top(), 0x0100_0000, "still the supervisor stack's");
        step(&mut interpreter);
        assert_eq!(interpreter.get_sp(), 0x8000);
        assert_eq!(interpreter.get_stack_top(), 0x8000, "the user stack's, where move usp put it");
        step(&mut interpreter);
        assert_eq!(interpreter.get_sp(), 0x7FFC);
        assert_eq!(interpreter.get_stack_top(), 0x8000);
        step(&mut interpreter);
        assert_eq!(interpreter.get_pc() as u32, label(&interpreter, "handler"));
        assert_eq!(interpreter.get_sp(), 0x00FF_FFFA);
        assert_eq!(
            interpreter.get_stack_top(),
            0x0100_0000,
            "the handler's stack holds its frame"
        );
        step(&mut interpreter);
        step(&mut interpreter);
        assert_eq!(interpreter.get_pc() as u32, label(&interpreter, "back"));
        assert_eq!(interpreter.get_sp(), 0x7FFC);
        assert_eq!(
            interpreter.get_stack_top(),
            0x8000,
            "the user stack's top, with the long it pushed under it"
        );
        //undo walks both tops back
        seen.pop();
        while let Some(before) = seen.pop() {
            interpreter.undo().unwrap();
            assert_eq!((interpreter.get_pc(), interpreter.get_stack_top()), before);
        }
        //a Poke of S swaps them as well, and its undo swaps them back
        for _ in 0..3 {
            interpreter.step().unwrap();
        }
        interpreter.begin_poke().unwrap();
        interpreter.set_sr(0x0000);
        assert!(interpreter.end_poke().unwrap());
        assert_eq!(interpreter.get_stack_top(), 0x8000, "the user stack's");
        interpreter.undo().unwrap();
        assert_eq!(interpreter.get_stack_top(), 0x0100_0000, "the supervisor stack's");
    }

    #[test]
    fn move_usp_reads_and_writes_the_user_stack_pointer() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #$12345678,a3
    move.l a3,usp
    move.l usp,a4
    simhalt
",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(interpreter.get_usp(), 0x1234_5678);
        assert_eq!(a(&interpreter, 4), 0x1234_5678);
        assert_eq!(
            a(&interpreter, 7),
            0x0100_0000,
            "A7 is the SSP, which it left alone"
        );
    }

    #[test]
    fn trap_15_works_in_user_mode() {
        let mut interpreter = machine(
            "    org $1000
start:
    andi.w #$dfff,sr
    move.l #42,d1
    move.b #3,d0
    trap #15
    simhalt
",
        );
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Interrupt));
        interpreter
            .answer_interrupt(InterruptResult::DisplayNumber)
            .unwrap();
        run_to_the_halt(&mut interpreter);
    }
}

mod nesting_and_double_faults {
    use super::*;

    pub(super) const NESTED: &str = "    org $1000
start:
    move.l #trap1,$84
    move.l #divide,$14
    trap #1
after:
    simhalt
trap1:
    moveq #0,d0
    divu d0,d1
    move.l a7,d3
    rte
divide:
    move.l a7,d2
    rte
";

    /// An exception inside a handler pushes its frame on top, and each `rte`
    /// returns to the handler it interrupted.
    #[test]
    fn an_exception_inside_a_handler_nests() {
        let mut interpreter = machine(NESTED);
        let divide = label(&interpreter, "divide") as usize;
        while interpreter.get_pc() != divide {
            interpreter.step().unwrap();
        }
        let frames: Vec<(FrameKind, Option<u8>)> = interpreter
            .get_pretty_call_stack()
            .iter()
            .map(|frame| (frame.kind, frame.vector))
            .collect();
        assert_eq!(
            frames,
            [
                (FrameKind::Exception, Some(33)),
                (FrameKind::Exception, Some(5))
            ]
        );
        assert_eq!(interpreter.get_pretty_call_stack()[1].label_name, "divide");
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 2), 0x0100_0000 - 12, "two frames deep");
        assert_eq!(
            d(&interpreter, 3),
            0x0100_0000 - 6,
            "one, after the first rte"
        );
        assert!(interpreter.get_pretty_call_stack().is_empty());
    }

    /// An odd supervisor stack pointer cannot take a frame: the processor
    /// halts, a double fault.
    #[test]
    fn a_frame_pushed_on_an_odd_stack_pointer_halts_the_processor() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #handler,$80
    lea $5001,a7
    trap #0
    simhalt
handler:
    rte
",
        );
        //the program counter's low word goes first, at the odd address below the pointer
        let fault = RuntimeError::AddressError {
            address: 0x5001 - 2,
            size: Size::Word,
            access: Access::Write,
        };
        assert_eq!(interpreter.run(), Err(fault.clone()));
        assert_eq!(
            *interpreter.get_status(),
            InterpreterStatus::TerminatedWithException
        );
        assert_eq!(
            interpreter.get_termination(),
            Some(&Termination::Halted {
                cause: fault,
                vector: 32,
            })
        );
    }

    /// With no handler the vector is read first, and the program ends with
    /// the exception before any frame is pushed, odd stack pointer or not.
    #[test]
    fn no_handler_comes_before_an_odd_stack_pointer() {
        let mut interpreter = machine("    org $1000\nstart:\n    lea $5001,a7\n    trap #0\n");
        assert_eq!(
            interpreter.run(),
            Err(RuntimeError::UnhandledTrap { vector: 32 })
        );
    }
}

mod trace {
    use super::*;

    /// T sampled before each instruction: `ori` that sets it is not traced,
    /// the instruction after it is; a `trap` traced pushes the trace frame on
    /// top of its own, so the trace handler runs first and returns into the
    /// trap handler, which is not traced; the `andi` that clears T is traced.
    #[test]
    fn trace_follows_every_instruction_that_started_with_t_set() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #trace,$24
    move.l #trap0,$80
    lea $8000,a6
    ori.w #$8000,sr
    nop
after_nop:
    trap #0
after_trap:
    andi.w #$7fff,sr
halt:
    simhalt
trace:
    addq.l #1,d7
    move.l 2(a7),(a6)+
    rte
trap0:
    move.l #1,d1
    rte
",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 7), 3, "three instructions were traced");
        assert_eq!(long(&interpreter, 0x8000), label(&interpreter, "after_nop"));
        assert_eq!(
            long(&interpreter, 0x8004),
            label(&interpreter, "trap0"),
            "the trap handler runs after the trace handler"
        );
        assert_eq!(long(&interpreter, 0x8008), label(&interpreter, "halt"));
        assert_eq!(d(&interpreter, 1), 1);
        assert_eq!(interpreter.get_sr() & 0x8000, 0);
    }

    /// An instruction that waits on the host is traced once it is answered.
    #[test]
    fn a_trap_15_task_is_traced_after_its_answer() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #trace,$24
    move.l #42,d1
    move.b #3,d0
    ori.w #$8000,sr
    trap #15
after:
    simhalt
trace:
    move.l 2(a7),d6
    andi.w #$7fff,(a7)
    rte
",
        );
        assert_eq!(interpreter.run(), Ok(InterpreterStatus::Interrupt));
        interpreter
            .answer_interrupt(InterruptResult::DisplayNumber)
            .unwrap();
        assert_eq!(interpreter.get_pc(), label(&interpreter, "trace") as usize);
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 6), label(&interpreter, "after"));
    }

    /// The trace frame after an answer is the processor's own, as after any
    /// other instruction: a 68010's format `$0` and a 68020's format `$2`,
    /// with the address of the `trap #15` traced, so that the handler's
    /// `rte` returns after it rather than raising a format error.
    #[test]
    fn a_trap_15_task_traced_after_its_answer_pushes_the_processors_frame() {
        for (processor, format, frame) in [("68010", 0x0024, 8), ("68020", 0x2024, 12)] {
            let mut interpreter = machine(&format!(
                "    opt p={processor}
    org $1000
start:
    move.l #trace,$24
    moveq #4,d0
    move.w #$A000,sr
traced:
    trap #15
after:
    simhalt
trace:
    move.w 6(a7),d5
    move.l 2(a7),d6
    move.l 8(a7),d4
    andi.w #$7fff,(a7)
    rte
"
            ));
            let top = a(&interpreter, 7);
            assert_eq!(interpreter.run(), Ok(InterpreterStatus::Interrupt));
            interpreter
                .answer_interrupt(InterruptResult::ReadNumber("7".to_string()))
                .unwrap();
            assert_eq!(interpreter.get_pc(), label(&interpreter, "trace") as usize);
            assert_eq!(a(&interpreter, 7), top - frame, "{processor}");
            run_to_the_halt(&mut interpreter);
            assert_eq!(d(&interpreter, 1), 7, "{processor}: the answer was written");
            assert_eq!(d(&interpreter, 5), format, "{processor}: the format word");
            assert_eq!(
                d(&interpreter, 6),
                label(&interpreter, "after"),
                "{processor}"
            );
            if processor == "68020" {
                assert_eq!(
                    d(&interpreter, 4),
                    label(&interpreter, "traced"),
                    "the instruction traced"
                );
            }
            assert_eq!(
                a(&interpreter, 7),
                top,
                "{processor}: `rte` popped the frame"
            );
        }
    }

    /// An instruction that raised an exception of group 1 did not run, and is
    /// not traced.
    #[test]
    fn an_illegal_instruction_is_not_traced() {
        let mut interpreter = machine(
            "    org $1000
start:
    move.l #trace,$24
    move.l #illegal,$10
    ori.w #$8000,sr
    illegal
    simhalt
trace:
    addq.l #1,d7
    rte
illegal:
    addq.l #1,d6
    addq.l #2,2(a7)
    andi.w #$7fff,(a7)
    rte
",
        );
        run_to_the_halt(&mut interpreter);
        assert_eq!(d(&interpreter, 6), 1);
        assert_eq!(d(&interpreter, 7), 0);
    }
}

mod the_history {
    use super::*;

    #[test]
    fn an_exception_entry_and_rte_are_journaled() {
        let mut interpreter = machine(&format!(
            "    org $1000
start:
    move.l #handler,$80
    trap #0
    simhalt
{RECORDING_HANDLER}"
        ));
        interpreter.step().unwrap();
        interpreter.step().unwrap();
        let step = interpreter.get_last_steps(1)[0].clone();
        assert!(step.get_mutations().iter().any(|mutation| matches!(
            mutation,
            MutationOperation::PushException {
                vector: 32,
                from: 0x1008,
                cause: "UnhandledTrap",
                vbr: 0,
                ..
            }
        )));
        assert_eq!(step.get_sr(), 0x2000);
        while interpreter.get_pc() != 0x100A {
            interpreter.step().unwrap();
        }
        let step = interpreter.get_last_steps(1)[0].clone();
        assert!(step.get_mutations().iter().any(|mutation| matches!(
            mutation,
            MutationOperation::PopException {
                to: 0x100A,
                frame: Some(_),
                ..
            }
        )));
    }

    /// An exception's entry in the history, and its frame of the call stack,
    /// say what raised it, which tells the two causes of a vector apart, the
    /// 68020's `trapcc` from `trapv` and its `chk2` from `chk`, and the
    /// vector base register it was taken with, which the handler can move
    /// before it returns.
    #[test]
    fn an_exception_says_what_raised_it_and_the_vbr_it_was_taken_with() {
        let mut interpreter = machine(
            "    opt p=68020
    org $1000
start:
    move.l #$8000,d0
    movec d0,vbr
    move.l #handler,$8000+4*6
    move.l #handler,$8000+4*7
    moveq #0,d1
    trapeq
    moveq #9,d1
    chk2.b bounds,d1
    simhalt
handler:
    move.l #$9000,d0
    movec d0,vbr
    simhalt
    move.l #$8000,d0
    movec d0,vbr
    rte
bounds:
    dc.b 0,5
",
        );
        for (vector, cause) in [(7, "ConditionalTrap"), (6, "Chk2OutOfBounds")] {
            run_to_the_halt(&mut interpreter);
            let frames = interpreter.get_pretty_call_stack();
            assert_eq!(frames.len(), 1, "{cause}");
            assert_eq!(frames[0].vector, Some(vector));
            assert_eq!(frames[0].cause, Some(cause));
            assert_eq!(frames[0].vbr, Some(0x8000), "{cause}: not the $9000 of now");
            let pushed = interpreter
                .get_last_steps(10)
                .iter()
                .flat_map(|step| step.get_mutations())
                .find_map(|mutation| match mutation {
                    MutationOperation::PushException {
                        vector, cause, vbr, ..
                    } => Some((*vector, *cause, *vbr)),
                    _ => None,
                });
            assert_eq!(pushed, Some((vector, cause, 0x8000)));
            let json = serde_json::to_value(&frames[0]).unwrap();
            assert_eq!(json["cause"], cause);
            assert_eq!(json["vbr"], 0x8000);
        }
        run_to_the_halt(&mut interpreter);
        assert!(interpreter.get_pretty_call_stack().is_empty());
    }

    /// The name an exception's cause is given is its `type` in TypeScript,
    /// the name serde gives the variant.
    #[test]
    fn a_cause_is_named_as_its_error_crosses() {
        for error in [
            RuntimeError::BusError {
                address: 0,
                size: Size::Word,
                access: Access::Read,
                range: crate::assembler::program::MemoryRangeKind::Invalid,
            },
            RuntimeError::AddressError {
                address: 1,
                size: Size::Word,
                access: Access::Read,
            },
            RuntimeError::IllegalInstruction,
            RuntimeError::DivisionByZero,
            RuntimeError::ChkOutOfBounds { value: 0, bound: 0 },
            RuntimeError::Chk2OutOfBounds {
                value: 0,
                lower: 0,
                upper: 0,
            },
            RuntimeError::OverflowException,
            RuntimeError::ConditionalTrap,
            RuntimeError::PrivilegeViolation,
            RuntimeError::Trace,
            RuntimeError::LineA(0xA000),
            RuntimeError::LineF(0xF000),
            RuntimeError::FormatError { format: 3 },
            RuntimeError::UnhandledTrap { vector: 32 },
            RuntimeError::Irq { level: 2 },
            RuntimeError::UnsupportedTrapTask { task: 200 },
        ] {
            let json = serde_json::to_value(&error).unwrap();
            assert_eq!(json["type"], error.name());
        }
    }

    /// A return to an odd address takes its frame off the call stack: the
    /// return has popped it, and the address error it raises in its own
    /// prefetch pushes the frame of vector 3 in its place. Undoing it puts the
    /// frame back.
    #[test]
    fn a_return_to_an_odd_address_takes_its_frame_off_the_call_stack() {
        let frames = |interpreter: &Interpreter| -> Vec<(FrameKind, Option<u8>)> {
            interpreter
                .get_pretty_call_stack()
                .iter()
                .map(|frame| (frame.kind, frame.vector))
                .collect()
        };
        for (name, setup, returns, before) in [
            (
                "rts",
                "    bsr sub
    simhalt
sub:
    move.l #$1001,(a7)
",
                "    rts",
                (FrameKind::Call, None),
            ),
            (
                "rtr",
                "    bsr sub
    simhalt
sub:
    move.l #$1001,(a7)
    move.w #0,-(a7)
",
                "    rtr",
                (FrameKind::Call, None),
            ),
            (
                "rte",
                "    move.l #trap_handler,$80
    trap #0
    simhalt
trap_handler:
    move.l #$1001,2(a7)
",
                "    rte",
                (FrameKind::Exception, Some(32)),
            ),
        ] {
            let mut interpreter = machine(&format!(
                "    org $1000
start:
    move.l #address_error,$c
{setup}returning:
{returns}
address_error:
    simhalt
"
            ));
            run_to_the_halt(&mut interpreter);
            assert_eq!(
                interpreter.get_pc() as u32,
                label(&interpreter, "address_error") + 4,
                "`{name}` faulted into the handler"
            );
            assert_eq!(
                frames(&interpreter),
                [(FrameKind::Exception, Some(3))],
                "`{name}` took its frame off"
            );
            //the simhalt, then the address error the return raised
            interpreter.undo().unwrap();
            interpreter.undo().unwrap();
            assert_eq!(interpreter.get_pc() as u32, label(&interpreter, "returning"));
            assert_eq!(frames(&interpreter), [before], "undoing `{name}` puts it back");
        }
    }

    #[test]
    fn a_change_of_s_journals_both_stack_pointers() {
        let mut interpreter = machine("    org $1000\nstart:\n    move.w #$0000,sr\n    simhalt\n");
        interpreter.step().unwrap();
        let step = interpreter.get_last_steps(1)[0].clone();
        assert!(step
            .get_mutations()
            .iter()
            .any(|mutation| matches!(mutation, MutationOperation::WriteStackPointers { .. })));
    }
}

mod undo {
    use super::*;

    #[test]
    fn undo_takes_back_an_exception_and_its_rte() {
        undo_restores_every_step(&format!(
            "    org $1000
start:
    move.l #handler,$14
    moveq #0,d1
    divu d1,d0
    simhalt
{RECORDING_HANDLER}"
        ));
    }

    #[test]
    fn undo_takes_back_user_mode_and_both_stacks() {
        undo_restores_every_step(
            "    org $1000
start:
    move.l #handler,$80
    move.l #$8000,a0
    move.l a0,usp
    andi.w #$dfff,sr
    move.l #1,-(a7)
    trap #0
    move.l (a7)+,d0
    simhalt
handler:
    move.l usp,a1
    move.l #$8100,a2
    move.l a2,usp
    move.l a1,usp
    rte
",
        );
    }

    #[test]
    fn undo_takes_back_rte_into_user_mode() {
        undo_restores_every_step(
            "    org $1000
start:
    move.l #$9000,a0
    move.l a0,usp
    pea user
    move.w #$0000,-(a7)
    rte
user:
    move.l a7,d0
    simhalt
",
        );
    }

    #[test]
    fn undo_takes_back_nested_exceptions_and_trace() {
        undo_restores_every_step(super::nesting_and_double_faults::NESTED);
        undo_restores_every_step(
            "    org $1000
start:
    move.l #trace,$24
    move.l #trap0,$80
    ori.w #$8000,sr
    nop
    trap #0
    andi.w #$7fff,sr
    simhalt
trace:
    rte
trap0:
    rte
",
        );
    }

    #[test]
    fn undo_takes_back_an_address_error_and_a_halt() {
        let steps = undo_restores_every_step(
            "    org $1000
start:
    move.l #handler,$c
    move.l #$8000,a0
    move.w (a0)+,d0
    move.l 1(a0),d1
done:
    simhalt
handler:
    move.l #done,10(a7)
    addq.l #8,a7
    rte
",
        );
        assert!(steps > 4);
        undo_restores_every_step(
            "    org $1000
start:
    move.l #handler,$80
    lea $5001,a7
    trap #0
handler:
    rte
",
        );
    }

    #[test]
    fn undo_takes_back_an_unhandled_exception() {
        undo_restores_every_step("    org $1000\nstart:\n    andi.w #$dfff,sr\n    rte\n");
    }
}

mod the_system_registers {
    use super::*;

    #[test]
    fn get_system_answers_sr_and_both_stack_pointers() {
        let interpreter = machine("    org $1000\nstart:\n    nop\n");
        let system = interpreter.get_system();
        assert_eq!(
            (system.sr, system.usp, system.ssp),
            (0x2000, 0x00FF_0000, 0x0100_0000)
        );
    }

    /// The System file's Pokes: each journaled, and one undo takes the whole
    /// Poke back, the stack swap of S included.
    #[test]
    fn a_poke_of_sr_usp_and_ssp_is_undone_as_one_step() {
        let mut interpreter = machine("    org $1000\nstart:\n    nop\n    nop\n");
        interpreter.step().unwrap();
        let before = snapshot(&interpreter);
        interpreter.begin_poke().unwrap();
        interpreter.set_usp(0x8000);
        interpreter.set_ssp(0x00FF_F000);
        interpreter.set_sr(0x0004);
        assert!(interpreter.end_poke().unwrap());
        assert_eq!(a(&interpreter, 7), 0x8000, "user mode, A7 the poked USP");
        assert_eq!(interpreter.get_ssp(), 0x00FF_F000);
        assert_eq!(interpreter.get_sr(), 0x0004);
        let step = interpreter.get_last_steps(1)[0].clone();
        let names: Vec<String> = step
            .get_writes()
            .iter()
            .map(|write| match write {
                crate::debugger::PokeWrite::Register { name, .. } => name.clone(),
                crate::debugger::PokeWrite::Memory { .. } => "memory".to_string(),
            })
            .collect();
        assert_eq!(names, ["usp", "ssp", "sr"]);
        interpreter.undo().unwrap();
        assert_eq!(snapshot(&interpreter), before);
    }

    #[test]
    fn a_write_outside_a_poke_is_direct_and_journals_nothing() {
        let mut interpreter = machine("    org $1000\nstart:\n    nop\n");
        interpreter.set_ssp(0x00FF_0100);
        interpreter.set_sr(0x2700);
        assert_eq!(a(&interpreter, 7), 0x00FF_0100);
        assert_eq!(interpreter.get_sr(), 0x2700);
        assert!(!interpreter.can_undo());
    }
}
